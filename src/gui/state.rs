//! What the window shows, and how events from the capture, the WebSocket server and the update
//! check change it.
//!
//! Everything here is plain data. Work that leaves the UI thread (saving files, talking to the
//! capture or the server) goes through [`Services`], which is `None` in headless snapshots and
//! tests, so every transition can run without a window or a network.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use reliquary_archiver::export::fribbels::Export;
use rune::{FileDialogOptions, FileDialogResult, Image, ImageCache, TextEdit, Tx};
use tracing::level_filters::LevelFilter;

use super::services::{self, Services};
use super::settings::{ImageFit, Settings};
use crate::session::SessionEvent;

/// A capture error that isn't repeated for this long has resolved itself
pub const CAPTURE_ERROR_TTL: Duration = Duration::from_secs(15);
/// How long a notice stays up
const NOTICE_TTL: Duration = Duration::from_secs(6);
/// How many of the most recent log lines the logs panel shows
pub const LOG_TAIL: usize = 400;
/// The image files a background can be chosen from
const BACKGROUND_EXTENSIONS: [&str; 7] = ["png", "jpg", "jpeg", "webp", "gif", "bmp", "svg"];

pub struct Ui {
    pub capture: Capture,
    /// Accounts in the order they first logged in
    pub accounts: Vec<Account>,
    /// The account exports and live sync are for. Follows the latest login unless the user picks one
    pub selected: Option<u32>,
    pub sync: LiveSync,
    pub update: Update,
    /// The update banner was dismissed for this run
    pub update_dismissed: bool,
    pub settings: Settings,
    /// The port field in the settings panel
    pub port_edit: TextEdit,
    /// A port that was asked for and hasn't been opened yet
    pub pending_port: Option<u16>,
    /// Why the port in the settings panel couldn't be used
    pub port_error: Option<String>,
    /// Why run on startup couldn't be changed
    pub run_on_start_error: Option<String>,
    pub panel: Option<Panel>,
    pub notice: Option<Notice>,
    pub log_level: LevelFilter,
    /// The most recent log lines, kept current while the logs panel is open
    pub log_tail: Vec<String>,
    /// A capture file is being read
    pub reading_file: bool,
    /// Capturing needs administrator rights the app doesn't have
    pub needs_admin: bool,
    /// There's a system tray icon to close the window to
    pub has_tray: bool,
    /// This build can update itself and wasn't told not to
    pub updates_supported: bool,
    pub services: Option<Services>,
    /// Decodes images off the UI thread, set up once the window's waker exists
    pub images: Option<ImageCache>,
    /// The background image and the path it was loaded from
    pub background: Option<(String, Image)>,
}

#[derive(Default)]
pub struct Capture {
    /// A packet arrived since the capture started or last failed
    pub traffic: bool,
    /// The game's connection is being read
    pub connected: bool,
    /// Data was missed and the rest of this login can't be read
    pub stalled: bool,
    /// The game's traffic can't be decrypted, so this version of the game isn't supported yet
    pub unsupported: bool,
    pub error: Option<CaptureError>,
}

pub struct CaptureError {
    pub message: String,
    pub at: Instant,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Inventory {
    pub relics: usize,
    pub light_cones: usize,
    pub characters: usize,
    pub materials: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Account {
    pub uid: u32,
    /// What the account holds, once all of its login data arrived
    pub inventory: Option<Inventory>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum LiveSync {
    #[default]
    Starting,
    Running {
        port: u16,
        clients: usize,
    },
    Failed(String),
    /// Closed while an update installs
    Closed,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Update {
    /// Not checked, or checking isn't possible on this platform
    #[default]
    Idle,
    Checking,
    UpToDate,
    Available {
        version: String,
    },
    Installing,
    // Only Windows builds update themselves
    #[cfg_attr(not(windows), allow(dead_code))]
    Failed(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Panel {
    Settings,
    Logs,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Notice {
    /// Tells notices apart, so an old one's timer doesn't dismiss a newer one
    pub id: u64,
    pub title: String,
    pub detail: Option<String>,
    pub error: bool,
    /// A file the notice can show in its folder
    pub file: Option<PathBuf>,
    pub until: Instant,
}

/// Where getting an account's data stands, which is what the window leads with
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    /// Nobody logged in yet
    Waiting,
    /// An account logged in, its data is still coming in
    Reading { uid: u32 },
    /// The account's data is complete and can be exported
    Ready { uid: u32, inventory: Inventory },
}

impl Ui {
    pub fn new(settings: Settings) -> Self {
        Self {
            capture: Capture::default(),
            accounts: Vec::new(),
            selected: None,
            sync: LiveSync::Starting,
            update: Update::Idle,
            update_dismissed: false,
            port_edit: TextEdit::new(settings.ws_port.to_string()),
            settings,
            pending_port: None,
            port_error: None,
            run_on_start_error: None,
            panel: None,
            notice: None,
            log_level: LevelFilter::INFO,
            log_tail: Vec::new(),
            reading_file: false,
            needs_admin: false,
            has_tray: false,
            updates_supported: false,
            services: None,
            images: None,
            background: None,
        }
    }

    pub fn stage(&self) -> Stage {
        match self.selected_account() {
            None => Stage::Waiting,
            Some(Account {
                uid,
                inventory: Some(inventory),
            }) => Stage::Ready {
                uid: *uid,
                inventory: *inventory,
            },
            Some(account) => Stage::Reading { uid: account.uid },
        }
    }

    pub fn selected_account(&self) -> Option<&Account> {
        self.accounts.iter().find(|a| Some(a.uid) == self.selected)
    }

    /// Changes the settings and saves them if that changed them
    pub fn settings_mut(&mut self, change: impl FnOnce(&mut Settings)) {
        change(&mut self.settings);
        if let Some(services) = &self.services {
            services.save_settings(&self.settings);
        }
    }

    pub fn notify(&mut self, title: impl Into<String>, detail: Option<String>, error: bool, file: Option<PathBuf>, tx: &mut Tx) {
        let id = self.notice.as_ref().map_or(0, |n| n.id + 1);
        self.notice = Some(Notice {
            id,
            title: title.into(),
            detail,
            error,
            file,
            until: Instant::now() + NOTICE_TTL,
        });
        tx.wake_after(NOTICE_TTL);
    }

    /// Drops what has timed out. True when that changed anything
    pub fn expire(&mut self, now: Instant) -> bool {
        let mut changed = false;
        if self.notice.as_ref().is_some_and(|n| n.until <= now) {
            self.notice = None;
            changed = true;
        }
        if self
            .capture
            .error
            .as_ref()
            .is_some_and(|e| now.duration_since(e.at) >= CAPTURE_ERROR_TTL)
        {
            self.capture.error = None;
            changed = true;
        }
        changed
    }

    // Capture

    pub fn traffic_seen(&mut self) {
        self.capture.traffic = true;
        self.capture.error = None;
    }

    pub fn capture_failed(&mut self, message: String, tx: &mut Tx) {
        self.capture.traffic = false;
        self.capture.error = Some(CaptureError {
            message,
            at: Instant::now(),
        });
        tx.wake_after(CAPTURE_ERROR_TTL);
    }

    /// Applies what the capture session made of a packet. True when that changed anything
    pub fn session_event(&mut self, event: &SessionEvent) -> bool {
        let capture = &mut self.capture;
        let before = (capture.connected, capture.stalled, capture.unsupported);
        match event {
            SessionEvent::ConnectionEstablished => {
                capture.connected = true;
                // A new login gets a new chance at decrypting, in case the game was updated in between
                capture.unsupported = false;
            }
            SessionEvent::Disconnected => capture.connected = false,
            SessionEvent::PacketsDecoded(n) if *n > 0 => capture.connected = true,
            SessionEvent::DecryptionKeyMissing => capture.unsupported = true,
            SessionEvent::Stalled => capture.stalled = true,
            SessionEvent::Recovered => capture.stalled = false,
            SessionEvent::PacketsDecoded(_)
            | SessionEvent::AccountSeen { .. }
            | SessionEvent::LoginFinished
            | SessionEvent::DecodeError => {}
        }
        before != (capture.connected, capture.stalled, capture.unsupported)
    }

    // Accounts

    /// An account logged in, for the first time or again. Logging in again starts its data over
    pub fn account_logged_in(&mut self, uid: u32) {
        match self.accounts.iter_mut().find(|a| a.uid == uid) {
            Some(account) => account.inventory = None,
            None => self.accounts.push(Account { uid, inventory: None }),
        }
        // Whoever logged in last is who the user is looking at in the game
        self.select(uid);
    }

    /// True when that changed anything
    pub fn inventory_changed(&mut self, uid: u32, inventory: Inventory) -> bool {
        let Some(account) = self.accounts.iter_mut().find(|a| a.uid == uid) else {
            return false;
        };
        let changed = account.inventory != Some(inventory);
        account.inventory = Some(inventory);
        changed
    }

    pub fn select(&mut self, uid: u32) {
        self.selected = Some(uid);
        if let Some(services) = &self.services {
            services.select_account(uid);
        }
    }

    // Live sync

    /// The server is now running on `port`
    pub fn sync_opened(&mut self, port: u16) {
        self.sync = LiveSync::Running { port, clients: 0 };
        self.port_error = None;
        // Only a port the server could open is saved. 0 stays 0, it asks for any free port every time
        if let Some(requested) = self.pending_port.take()
            && (requested == 0 || requested == port)
        {
            self.settings_mut(|s| s.ws_port = requested);
        }
    }

    pub fn sync_failed(&mut self, error: String) {
        let port = self.pending_port.unwrap_or(self.settings.ws_port);
        let error = if error.starts_with("Address already in use") {
            format!("Port {port} is already in use. Close other copies of the archiver, or choose another port.")
        } else {
            error
        };
        // A server that was already running keeps running on its old port
        if !matches!(self.sync, LiveSync::Running { .. }) {
            self.sync = LiveSync::Failed(error.clone());
        }
        if self.pending_port.take().is_some() || matches!(self.sync, LiveSync::Failed(_)) {
            self.port_error = Some(error);
        }
    }

    pub fn sync_clients(&mut self, count: usize) {
        if let LiveSync::Running { clients, .. } = &mut self.sync {
            *clients = count;
        }
    }

    /// Moves the server to the port typed into the settings panel
    pub fn apply_port(&mut self) {
        let text = self.port_edit.text.trim();
        let Ok(port) = text.parse::<u16>() else {
            self.port_error = Some(format!("\"{text}\" isn't a port, use a number from 0 to 65535"));
            return;
        };
        if matches!(self.sync, LiveSync::Running { port: running, .. } if running == port) {
            self.port_error = None;
            return;
        }
        self.port_error = None;
        self.pending_port = Some(port);
        if let Some(services) = &self.services {
            services.open_port(port);
        }
    }

    pub fn port_edited(&self) -> bool {
        match self.sync {
            LiveSync::Running { port, .. } => self.port_edit.text.trim() != port.to_string(),
            _ => true,
        }
    }

    // Updates

    #[cfg_attr(not(windows), allow(dead_code))]
    pub fn update_checked(&mut self, result: Result<Option<String>, String>) {
        self.update = match result {
            Ok(Some(version)) => Update::Available { version },
            Ok(None) => Update::UpToDate,
            Err(error) => Update::Failed(error),
        };
        if matches!(self.update, Update::Available { .. }) && self.settings.always_update {
            self.install_update();
        }
    }

    pub fn check_for_update(&mut self) {
        if let Some(services) = &self.services
            && self.updates_supported
        {
            self.update = Update::Checking;
            services.check_for_update();
        }
    }

    pub fn install_update(&mut self) {
        let Some(services) = &self.services else {
            return;
        };
        self.update = Update::Installing;
        // The new version opens the server again, and it can't while this one holds the port
        self.sync = LiveSync::Closed;
        services.close_port();
        services.install_update();
    }

    /// The update didn't install, so this version carries on
    #[cfg_attr(not(windows), allow(dead_code))]
    pub fn update_failed(&mut self, error: String) {
        self.update = Update::Failed(error);
        self.sync = LiveSync::Starting;
        if let Some(services) = &self.services {
            services.open_port(self.settings.ws_port);
        }
    }

    // Files

    /// Asks where to save the selected account's export, then saves it there
    pub fn save_export(&mut self, tx: &mut Tx) {
        let Stage::Ready { uid, .. } = self.stage() else {
            return;
        };
        tx.save_file(services::export_dialog(), move |s: &mut Ui, result, tx| {
            if let Some(path) = s.dialog_path(result, tx)
                && let Some(services) = &s.services
            {
                services.save_export(uid, path);
            }
        });
    }

    pub fn open_capture_file(&mut self, tx: &mut Tx) {
        tx.open_file(services::capture_file_dialog(), |s: &mut Ui, result, tx| {
            if let Some(path) = s.dialog_path(result, tx)
                && let Some(services) = &s.services
            {
                s.reading_file = true;
                services.read_capture_file(path);
            }
        });
    }

    /// A capture file was read, `export` is the account found in it
    pub fn capture_file_read(&mut self, export: Option<Box<Export>>, tx: &mut Tx) {
        self.reading_file = false;
        let Some(export) = export else {
            let detail = "It has no complete login in it. Capture from the title screen until the game finished loading.";
            self.notify("No account found in that file", Some(detail.into()), true, None, tx);
            return;
        };
        tx.save_file(services::export_dialog(), move |s: &mut Ui, result, tx| {
            if let Some(path) = s.dialog_path(result, tx)
                && let Some(services) = &s.services
            {
                services.save_file_export(*export, path);
            }
        });
    }

    pub fn save_logs(&mut self, tx: &mut Tx) {
        let options = FileDialogOptions {
            title: "Save logs".into(),
            file_name: Some("reliquary-archiver-log.txt".into()),
            ..Default::default()
        };
        tx.save_file(options, |s: &mut Ui, result, tx| {
            if let Some(path) = s.dialog_path(result, tx)
                && let Some(services) = &s.services
            {
                services.save_logs(path);
            }
        });
    }

    /// The path a file dialog was answered with, if it wasn't cancelled
    fn dialog_path(&mut self, result: FileDialogResult, tx: &mut Tx) -> Option<PathBuf> {
        match result {
            Ok(path) => path,
            Err(e) => {
                self.notify("Couldn't show the file dialog", Some(e.to_string()), true, None, tx);
                None
            }
        }
    }

    pub fn set_run_on_start(&mut self, enabled: bool) {
        let Some(services) = &self.services else {
            return;
        };
        match services.set_run_on_start(enabled) {
            Ok(()) => {
                self.run_on_start_error = None;
                self.settings_mut(|s| s.run_on_start = enabled);
            }
            Err(e) => {
                tracing::warn!(%e, "failed to change run on startup");
                self.run_on_start_error = Some(e);
            }
        }
    }

    // Background

    /// Loads the background image the settings name, if it isn't loaded already
    pub fn load_background(&mut self) {
        let path = &self.settings.background_image;
        if path.is_empty() {
            self.background = None;
            return;
        }
        if self.background.as_ref().is_some_and(|(loaded, _)| loaded == path) {
            return;
        }
        let Some(images) = &self.images else {
            return;
        };
        // Raster images decode at their own size, the size arguments only matter for SVGs
        let image = images.file(path, 1024.0, 1.0);
        self.background = Some((path.clone(), image));
    }

    pub fn choose_background(&mut self, tx: &mut Tx) {
        let options = FileDialogOptions {
            title: "Choose a background image".into(),
            ..Default::default()
        }
        .filter("Images", &BACKGROUND_EXTENSIONS);
        tx.open_file(options, |s: &mut Ui, result, tx| {
            if let Some(path) = s.dialog_path(result, tx) {
                s.settings_mut(|settings| settings.background_image = path.to_string_lossy().into_owned());
                s.load_background();
            }
        });
    }

    pub fn remove_background(&mut self) {
        self.settings_mut(|s| s.background_image.clear());
        self.background = None;
    }

    pub fn set_image_fit(&mut self, fit: ImageFit) {
        self.settings_mut(|s| s.image_fit = fit);
    }

    pub fn set_background_opacity(&mut self, opacity: f32) {
        // Whole percents, so dragging doesn't save a new file for every pixel moved
        let opacity = (opacity.clamp(0.0, 1.0) * 100.0).round() / 100.0;
        if opacity != self.settings.background_opacity {
            self.settings_mut(|s| s.background_opacity = opacity);
        }
    }

    /// The image behind the window, when there is one that decoded
    pub fn background_image(&self) -> Option<&Image> {
        self.background
            .as_ref()
            .map(|(_, image)| image)
            .filter(|image| image.error().is_none())
    }

    // Panels

    pub fn toggle_panel(&mut self, panel: Panel) {
        self.panel = if self.panel == Some(panel) { None } else { Some(panel) };
        if self.panel == Some(Panel::Logs) {
            self.refresh_log_tail();
        }
        if self.panel == Some(Panel::Settings) {
            // The field starts from what is in use, not from whatever was left typed in it
            self.port_edit.replace_text(self.settings.ws_port.to_string());
            self.port_error = None;
        }
    }

    pub fn refresh_log_tail(&mut self) {
        let lines = crate::logging::lines();
        self.log_tail = lines[lines.len().saturating_sub(LOG_TAIL)..].to_vec();
    }

    pub fn set_log_level(&mut self, level: LevelFilter) {
        self.log_level = level;
        crate::logging::set_console_level(level);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ui() -> Ui {
        Ui::new(Settings::default())
    }

    #[test]
    fn the_latest_login_is_selected_and_its_data_starts_over() {
        let mut ui = ui();
        ui.account_logged_in(1);
        ui.inventory_changed(
            1,
            Inventory {
                relics: 3,
                ..Default::default()
            },
        );
        ui.account_logged_in(2);
        assert_eq!(ui.stage(), Stage::Reading { uid: 2 });

        ui.account_logged_in(1);
        assert_eq!(ui.stage(), Stage::Reading { uid: 1 });
        assert_eq!(ui.accounts.iter().map(|a| a.uid).collect::<Vec<_>>(), [1, 2]);
    }

    #[test]
    fn an_account_is_ready_once_its_inventory_arrives() {
        let mut ui = ui();
        assert_eq!(ui.stage(), Stage::Waiting);
        ui.account_logged_in(7);
        let inventory = Inventory {
            relics: 1,
            light_cones: 2,
            characters: 3,
            materials: 4,
        };
        assert!(ui.inventory_changed(7, inventory));
        assert!(!ui.inventory_changed(7, inventory));
        assert_eq!(ui.stage(), Stage::Ready { uid: 7, inventory });
    }

    #[test]
    fn inventories_of_unknown_accounts_are_ignored() {
        let mut ui = ui();
        assert!(!ui.inventory_changed(7, Inventory::default()));
        assert!(ui.accounts.is_empty());
    }

    #[test]
    fn session_events_report_whether_they_changed_anything() {
        let mut ui = ui();
        assert!(ui.session_event(&SessionEvent::PacketsDecoded(3)));
        assert!(!ui.session_event(&SessionEvent::PacketsDecoded(5)));
        assert!(!ui.session_event(&SessionEvent::DecodeError));
        assert!(ui.session_event(&SessionEvent::Stalled));
        assert!(ui.capture.stalled);
        assert!(ui.session_event(&SessionEvent::Recovered));
        assert!(ui.session_event(&SessionEvent::Disconnected));
        assert!(!ui.capture.connected);
    }

    #[test]
    fn a_new_connection_forgets_that_decryption_failed() {
        let mut ui = ui();
        ui.session_event(&SessionEvent::DecryptionKeyMissing);
        assert!(ui.capture.unsupported);
        ui.session_event(&SessionEvent::ConnectionEstablished);
        assert!(!ui.capture.unsupported);
    }

    #[test]
    fn a_requested_port_is_saved_once_the_server_runs_on_it() {
        let mut ui = ui();
        ui.sync_opened(23313);
        ui.port_edit.replace_text("4000");
        ui.apply_port();
        assert_eq!(ui.pending_port, Some(4000));
        assert_eq!(ui.settings.ws_port, 23313);

        ui.sync_opened(4000);
        assert_eq!(ui.settings.ws_port, 4000);
        assert_eq!(ui.pending_port, None);
    }

    #[test]
    fn port_zero_is_saved_as_zero() {
        let mut ui = ui();
        ui.port_edit.replace_text("0");
        ui.apply_port();
        ui.sync_opened(51234);
        assert_eq!(ui.settings.ws_port, 0);
        assert_eq!(ui.sync, LiveSync::Running { port: 51234, clients: 0 });
    }

    #[test]
    fn a_port_that_cant_be_opened_keeps_the_running_server() {
        let mut ui = ui();
        ui.sync_opened(23313);
        ui.port_edit.replace_text("80");
        ui.apply_port();
        ui.sync_failed("Address already in use, please close any other instances".into());
        assert_eq!(ui.sync, LiveSync::Running { port: 23313, clients: 0 });
        assert_eq!(
            ui.port_error.as_deref(),
            Some("Port 80 is already in use. Close other copies of the archiver, or choose another port.")
        );
        assert_eq!(ui.settings.ws_port, 23313);
    }

    #[test]
    fn a_server_that_never_started_reports_why() {
        let mut ui = ui();
        ui.sync_failed("Address already in use, please close any other instances".into());
        let expected = "Port 23313 is already in use. Close other copies of the archiver, or choose another port.";
        assert_eq!(ui.sync, LiveSync::Failed(expected.into()));
        ui.sync_failed("permission denied".into());
        assert_eq!(ui.sync, LiveSync::Failed("permission denied".into()));
    }

    #[test]
    fn text_that_isnt_a_port_is_refused() {
        let mut ui = ui();
        ui.port_edit.replace_text("70000");
        ui.apply_port();
        assert!(ui.port_error.is_some());
        assert_eq!(ui.pending_port, None);
    }

    #[test]
    fn client_counts_only_apply_to_a_running_server() {
        let mut ui = ui();
        ui.sync_clients(2);
        assert_eq!(ui.sync, LiveSync::Starting);
        ui.sync_opened(1);
        ui.sync_clients(2);
        assert_eq!(ui.sync, LiveSync::Running { port: 1, clients: 2 });
    }

    #[test]
    fn background_opacity_is_kept_in_whole_percents_between_0_and_1() {
        let mut ui = ui();
        ui.set_background_opacity(0.4567);
        assert_eq!(ui.settings.background_opacity, 0.46);
        ui.set_background_opacity(3.0);
        assert_eq!(ui.settings.background_opacity, 1.0);
        ui.set_background_opacity(-1.0);
        assert_eq!(ui.settings.background_opacity, 0.0);
    }

    #[test]
    fn removing_the_background_forgets_its_path() {
        let mut ui = ui();
        ui.settings.background_image = "wallpaper.png".into();
        ui.remove_background();
        assert!(ui.settings.background_image.is_empty());
        assert!(ui.background.is_none());
    }

    #[test]
    fn settings_from_before_the_rewrite_keep_their_background_and_tray_choice() {
        let old = r#"{"background_image":"C:\\bg.png","image_fit":"Contain","background_opacity":0.3,"text_shadow_enabled":true,
            "always_update":false,"minimize_to_tray_on_close":true,"minimize_to_tray_on_minimize":true,"run_on_start":false,
            "start_minimized":false,"ws_port":4000}"#;
        let settings: Settings = serde_json::from_str(old).unwrap();
        assert_eq!(settings.background_image, "C:\\bg.png");
        assert_eq!(settings.image_fit, ImageFit::Contain);
        assert_eq!(settings.background_opacity, 0.3);
        assert!(settings.close_to_tray);
        assert_eq!(settings.ws_port, 4000);
    }

    #[test]
    fn capture_errors_and_notices_expire() {
        let mut ui = ui();
        let now = Instant::now();
        ui.capture.error = Some(CaptureError {
            message: "no npcap".into(),
            at: now,
        });
        assert!(!ui.expire(now));
        assert!(ui.expire(now + CAPTURE_ERROR_TTL));
        assert!(ui.capture.error.is_none());
    }
}
