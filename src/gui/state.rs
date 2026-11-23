//! What the window shows, and how events from the capture, the WebSocket server and the update
//! check change it.
//!
//! Everything here is plain data. Work that leaves the UI thread (saving files, talking to the
//! capture or the server) goes through [`Services`], which is `None` in headless snapshots and
//! tests, so every transition can run without a window or a network.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use reliquary_archiver::export::fribbels::Export;
use rune::{FileDialogOptions, FileDialogResult, Image, ImageCache, TextEdit, Tx};
use tracing::level_filters::LevelFilter;

use super::background;
use super::services::{self, Services};
use super::settings::{BackgroundFill, ImageFit, Settings};
use crate::session::SessionEvent;
use crate::warps::Pool;
use crate::warps::service::{Command as WarpCommand, ProfileChoice, Summary, Update as WarpUpdate};

/// A capture error that isn't repeated for this long has resolved itself
pub const CAPTURE_ERROR_TTL: Duration = Duration::from_secs(15);
/// How long a notice stays up
const NOTICE_TTL: Duration = Duration::from_secs(6);
/// How many of the most recent log lines the logs panel shows
pub const LOG_TAIL: usize = 400;
/// The image files a background can be chosen from
const BACKGROUND_EXTENSIONS: [&str; 7] = ["png", "jpg", "jpeg", "webp", "gif", "bmp", "svg"];

pub struct Ui {
    /// The main window or the warp screen, which takes it over
    pub screen: Screen,
    /// Character and light cone art, set up with the window
    pub art: Option<super::art::ArtCache>,
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
    /// The average colour along each edge of the background image (top, right, bottom, left), for
    /// [`BackgroundFill::EdgeColor`]; none for vector images
    pub background_edges: Option<[rune::Color; 4]>,
    /// Gets background images ready off the UI thread, set up with the window
    pub background_loader: Option<rune::Latest<(String, Result<background::Prepared, String>)>>,
    /// The background the loader is working on
    pub background_loading: Option<String>,
    /// Why the background couldn't be read
    pub background_error: Option<String>,
    pub warps: Warps,
}

/// The warp tracker: what the warp database reports, and the warp screen's own state
pub struct Warps {
    /// Why the database couldn't be opened
    pub unavailable: Option<String>,
    /// Accounts with warps kept, the latest first
    pub accounts: Vec<u32>,
    pub summaries: HashMap<u32, Summary>,
    /// What the database is busy with, e.g. a sync
    pub busy: Option<String>,
    /// The pool the warp screen shows
    pub pool: Pool,
    /// The warp screen's view of it: its banner, or scrolled down to its history
    pub page: WarpPage,
    /// Wheel and trackpad scrolls on their way to turning the warp screen's page
    pub snap: Snap,
    /// Where each page's own scroll is, by [`WarpPage`]: its offset and the most it scrolls
    pub page_scroll: [rune::Live<[f32; 2]>; 2],
    /// A link to the warp history, pasted in
    pub link_edit: TextEdit,
    /// A backup with several profiles waiting for the user to pick one
    pub choose: Option<ProfilePick>,
    /// The featured 5 star picked on each pool's running banners
    pub featured: HashMap<Pool, u32>,
    /// What each account owns, from its login this run
    pub rosters: HashMap<u32, Roster>,
    /// When this run started, in Unix seconds: pulls seen live since make up the session
    pub session_start: i64,
    /// The ticket last picked on the warp screen's rail, and when: its shine plays for a moment
    /// after, and not again when the layout changes
    pub picked: Option<(Pool, Instant)>,
}

/// The warp screen's two views of a warp, one over the other: what its banner is and what the
/// warps in hand buy, and below it the whole history
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum WarpPage {
    #[default]
    Banner,
    History,
}

/// How far a scroll pulls past a page's end before the page turns, in points
const SNAP_PULL: f32 = 60.0;
/// A pause this long between scrolls starts the pull over, in seconds
const SNAP_PAUSE: f64 = 0.22;
/// After a turn, scrolls are held this long, so the rest of the gesture doesn't run on into the
/// new page or turn it back, in seconds
const SNAP_SETTLE: f64 = 0.55;

/// Turns the warp screen's page with the wheel, as a page that snaps rather than scrolls: a scroll
/// that pulls far enough past the end of a page goes to the next
#[derive(Clone, Copy, Debug, Default)]
pub struct Snap {
    /// How far the scrolls under way pull, down negative
    pull: f32,
    /// When the last scroll came and the page last turned, on Rune's clock ([`Tx::clock`])
    last: Option<f64>,
    turned: Option<f64>,
}

/// Where a page's own scroll stands, for whether a scroll past it turns the page
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PageScroll {
    pub at_top: bool,
    pub at_bottom: bool,
}

impl PageScroll {
    /// From the offset and the most it scrolls, as the page reports them
    pub fn of([offset, max]: [f32; 2]) -> PageScroll {
        PageScroll {
            at_top: offset <= 1.0,
            at_bottom: offset >= max - 1.0,
        }
    }
}

/// A wheel or trackpad scroll `dy` points over the page (down negative, as Rune gives it). True
/// takes it: it turned the page, or pulls towards turning it, or comes too soon after a turn.
/// False lets the page scroll itself. `now` is Rune's clock, in seconds
pub fn snap_wheel(s: &mut Ui, dy: f32, notch: bool, momentum: bool, scroll: PageScroll, now: f64) -> bool {
    let snap = &mut s.warps.snap;
    if snap.turned.is_some_and(|at| now - at < SNAP_SETTLE) {
        return true;
    }
    if dy == 0.0 {
        return false;
    }
    let (down, page) = (dy < 0.0, s.warps.page);
    let past_end = match page {
        WarpPage::Banner => down && scroll.at_bottom,
        WarpPage::History => !down && scroll.at_top,
    };
    if !past_end || momentum {
        snap.pull = 0.0;
        return false;
    }
    if snap.last.is_none_or(|at| now - at > SNAP_PAUSE) || snap.pull.signum() != dy.signum() {
        snap.pull = 0.0;
    }
    snap.pull += dy;
    snap.last = Some(now);
    // A notch of a mouse wheel is a step, it turns the page at once
    if notch || snap.pull.abs() >= SNAP_PULL {
        *snap = Snap {
            turned: Some(now),
            ..Snap::default()
        };
        turn_page(s);
    }
    true
}

/// Shows the warp screen's other page
pub fn turn_page(s: &mut Ui) {
    s.warps.page = match s.warps.page {
        WarpPage::Banner => WarpPage::History,
        WarpPage::History => WarpPage::Banner,
    };
}

/// How long a ticket picked on the warp screen's rail shines: the shine's delay and sweep, and a
/// little over
pub const SHINE_FOR: Duration = Duration::from_millis(600);

/// Picks a warp on the warp screen's rail: it shows, and its ticket shines for a moment
pub fn pick_warp(s: &mut Ui, pool: Pool, tx: &mut Tx) {
    s.warps.pool = pool;
    s.warps.picked = Some((pool, Instant::now()));
    tx.wake_after(SHINE_FOR);
}

/// The characters and light cones an account owns
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Roster {
    /// Character id to eidolon
    pub eidolons: HashMap<u32, u32>,
    /// Light cone id to its highest superimposition
    pub superimpositions: HashMap<u32, u32>,
}

impl Roster {
    /// Copies owned: a character at E2 is 3, a light cone at S1 is 1
    pub fn copies(&self, item: u32) -> u32 {
        if let Some(e) = self.eidolons.get(&item) {
            return e + 1;
        }
        self.superimpositions.get(&item).copied().unwrap_or(0)
    }
}

pub struct ProfilePick {
    pub path: PathBuf,
    pub uid: u32,
    pub profiles: Vec<ProfileChoice>,
}

impl Default for Warps {
    fn default() -> Self {
        Self {
            unavailable: None,
            accounts: Vec::new(),
            summaries: HashMap::new(),
            busy: None,
            pool: Pool::Character,
            page: WarpPage::Banner,
            snap: Snap::default(),
            page_scroll: [rune::Live::new([0.0; 2]), rune::Live::new([0.0; 2])],
            featured: HashMap::new(),
            rosters: HashMap::new(),
            session_start: chrono::Utc::now().timestamp(),
            picked: None,
            link_edit: TextEdit::new(""),
            choose: None,
        }
    }
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
    pub funds: Funds,
}

/// What the account has to warp with
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Funds {
    pub stellar_jade: u32,
    /// Star Rail Passes, for the standard warp
    pub passes: u32,
    /// Star Rail Special Passes, for event warps
    pub special_passes: u32,
    pub undying_starlight: u32,
}

/// What the window shows
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Screen {
    #[default]
    Main,
    /// The warp screen, over the whole window
    Warps,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Account {
    pub uid: u32,
    /// What the account holds, once all of its login data arrived
    pub inventory: Option<Inventory>,
    /// Its data was picked up from the last run, so what changed while the archiver was closed
    /// is missing until it logs in again
    pub picked_up: bool,
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
            screen: Screen::Main,
            art: None,
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
            background_loader: None,
            background_edges: None,
            background_loading: None,
            background_error: None,
            warps: Warps::default(),
        }
    }

    pub fn stage(&self) -> Stage {
        match self.selected_account() {
            None => Stage::Waiting,
            Some(Account {
                uid,
                inventory: Some(inventory),
                ..
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
        // A picked ticket's shine has swept: it goes, so nothing of it lingers at rest
        if self.warps.picked.is_some_and(|(_, at)| now.duration_since(at) >= SHINE_FOR) {
            self.warps.picked = None;
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
        if let SessionEvent::AccountSeen { uid } | SessionEvent::PickedUp { uid } = *event {
            // The session reports these in order, the login a pick-up starts from first
            let picked_up = matches!(event, SessionEvent::PickedUp { .. });
            return match self.accounts.iter_mut().find(|a| a.uid == uid) {
                Some(account) => std::mem::replace(&mut account.picked_up, picked_up) != picked_up,
                // Its AccountLoggedIn is still on its way, which keeps what's set here
                None if picked_up => {
                    self.accounts.push(Account {
                        uid,
                        inventory: None,
                        picked_up,
                    });
                    true
                }
                None => false,
            };
        }

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
            | SessionEvent::PickedUp { .. }
            | SessionEvent::LoginFinished
            | SessionEvent::DecodeError
            | SessionEvent::Warp(_) => {}
        }
        before != (capture.connected, capture.stalled, capture.unsupported)
    }

    // Accounts

    /// An account logged in, for the first time or again. Logging in again starts its data over
    pub fn account_logged_in(&mut self, uid: u32) {
        match self.accounts.iter_mut().find(|a| a.uid == uid) {
            Some(account) => account.inventory = None,
            None => self.accounts.push(Account {
                uid,
                inventory: None,
                picked_up: false,
            }),
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
        self.warp_command(WarpCommand::Show(uid));
    }

    // Warps

    /// The account the warp tracker shows: the selected one, or the one with the latest warps
    pub fn warps_uid(&self) -> Option<u32> {
        self.selected.or_else(|| self.warps.accounts.first().copied())
    }

    pub fn warps_summary(&self) -> Option<&Summary> {
        self.warps.summaries.get(&self.warps_uid()?)
    }

    /// What the account the warp tracker shows has to warp with, when it logged in this run
    pub fn warp_funds(&self) -> Option<Funds> {
        let uid = self.warps_uid()?;
        let account = self.accounts.iter().find(|a| a.uid == uid)?;
        Some(account.inventory?.funds)
    }

    pub fn open_warps(&mut self) {
        self.screen = Screen::Warps;
        self.panel = None;
        self.warps.page = WarpPage::Banner;
        // Opens on the pool pulled on last
        if let Some(summary) = self.warps_summary()
            && let Some(latest) = summary
                .pools
                .iter()
                .filter_map(|p| p.recent.first().map(|(pull, _, _)| (pull.sort_key(), p.pool)))
                .max()
        {
            self.warps.pool = latest.1;
        }
    }

    /// What the account the warp screen shows owns, when it logged in this run
    pub fn warp_roster(&self) -> Option<&Roster> {
        self.warps.rosters.get(&self.warps_uid()?)
    }

    pub fn roster_changed(&mut self, uid: u32, roster: Roster) -> bool {
        if self.warps.rosters.get(&uid) == Some(&roster) {
            return false;
        }
        self.warps.rosters.insert(uid, roster);
        true
    }

    pub fn close_warps(&mut self) {
        self.screen = Screen::Main;
    }

    /// Hands art that arrived to Rune. True when there was any
    pub fn art_ready(&mut self) -> bool {
        let arrived = self.art.as_ref().is_some_and(|art| art.take_done());
        if arrived {
            // The banner art's manifest may be what arrived, which says where the tickets are
            self.preload_warp_art();
        }
        arrived
    }

    /// Asks for everything the warp screen shows, as soon as the warps and the banner art's manifest
    /// are in, so the screen opens with it ready. In the order it's seen: every warp's tickets, plain
    /// and selected; each warp's own art (its banner's layers, or a portrait) and preview; then the
    /// other banners that can be picked, and the pass icons. Asking for art that's here or on its
    /// way is free, so this runs again whenever something arrives
    pub fn preload_warp_art(&self) {
        use super::art::{Art, Layer, Ticket};
        let (Some(art), Some(summary)) = (self.art.as_ref(), self.warps_summary()) else {
            return;
        };
        // Every warp's ticket, and every running banner's, which the pick switches between
        let tickets = summary
            .pools
            .iter()
            .filter_map(|stats| Ticket::of(summary, stats.pool, None))
            .chain(
                summary
                    .banners
                    .iter()
                    .flat_map(|b| b.fives.iter().map(|&item| Ticket::Banner(item))),
            );
        for ticket in tickets {
            for selected in [false, true] {
                let _ = art.get(Art::Ticket(ticket, selected));
            }
        }
        let shown = |item: u32| {
            if art
                .banner(item)
                .is_some_and(|banner| banner.surface.as_ref().is_some_and(|s| s.pattern.is_some()))
            {
                let _ = art.get(Art::Pattern(item));
            }
            match art.banner(item).filter(|banner| banner.placed()) {
                Some(banner) => {
                    for layer in [Layer::Backdrop, Layer::Back, Layer::Main, Layer::Front, Layer::Front2] {
                        if banner.layer(layer).is_some() {
                            let _ = art.get(Art::Layer(item, layer));
                        }
                    }
                }
                None => {
                    let _ = art.get(Art::Portrait(item));
                }
            }
            let _ = art.get(Art::Preview(item));
        };
        // Each warp's own: what its newest banner features, or its last 5 star
        for stats in &summary.pools {
            let own = summary
                .running(stats.pool)
                .next()
                .and_then(|banner| banner.fives.first().copied())
                .or_else(|| stats.fives.first().map(|five| five.item_id));
            if let Some(item) = own {
                shown(item);
            }
        }
        for banner in &summary.banners {
            for &item in &banner.fives {
                shown(item);
            }
        }
        for pass in [101, 102] {
            let _ = art.get(Art::Item(pass));
        }
    }

    fn warp_command(&self, command: WarpCommand) {
        if let Some(services) = &self.services {
            services.warps.send(command);
        }
    }

    /// Takes what the warp database reports
    pub fn warp_update(&mut self, update: WarpUpdate, tx: &mut Tx) {
        // What the warp database reports changes what's shown: let it come in on its transitions,
        // which Rune skips for changes no input caused
        if matches!(update, WarpUpdate::Summary(_) | WarpUpdate::Accounts(_)) {
            tx.animated();
        }
        match update {
            WarpUpdate::Unavailable(error) => self.warps.unavailable = Some(error),
            WarpUpdate::Accounts(accounts) => {
                self.warps.accounts = accounts;
                if let Some(uid) = self.warps_uid()
                    && !self.warps.summaries.contains_key(&uid)
                {
                    self.warp_command(WarpCommand::Show(uid));
                }
            }
            WarpUpdate::Summary(summary) => {
                // A pool the account never pulled on drops out of the tabs
                if summary.uid == self.warps_uid().unwrap_or(summary.uid) && summary.pool(self.warps.pool).is_none() {
                    self.warps.pool = Pool::Character;
                }
                self.warps.summaries.insert(summary.uid, *summary);
                self.preload_warp_art();
            }
            WarpUpdate::Busy(busy) => self.warps.busy = busy,
            WarpUpdate::Notice { title, detail, error } => self.notify(title, detail, error, None, tx),
            WarpUpdate::ChooseProfile { path, uid, profiles } => {
                self.warps.choose = Some(ProfilePick { path, uid, profiles });
                // The pick is on the warp screen's history page, under where backups are imported
                self.screen = Screen::Warps;
                self.warps.page = WarpPage::History;
            }
        }
    }

    pub fn sync_warps(&mut self) {
        if let Some(uid) = self.warps_uid() {
            self.warps.busy = Some("Syncing…".into());
            self.warp_command(WarpCommand::Sync(uid));
        }
    }

    pub fn find_cached_key(&mut self) {
        self.warps.busy = Some("Looking in the game's cache…".into());
        self.warp_command(WarpCommand::FindCachedKey);
    }

    pub fn use_history_link(&mut self) {
        let link = self.warps.link_edit.text.trim().to_string();
        if link.is_empty() {
            return;
        }
        self.warps.link_edit.replace_text("");
        self.warps.busy = Some("Checking the link…".into());
        self.warp_command(WarpCommand::UseLink(link));
    }

    /// Asks for a Star Rail Station backup, then imports it into the account the tracker shows
    pub fn import_srs(&mut self, tx: &mut Tx) {
        let Some(uid) = self.warps_uid() else {
            return;
        };
        let options = FileDialogOptions {
            title: "Import a Star Rail Station backup".into(),
            ..Default::default()
        }
        .filter("Star Rail Station backups", &["dat"]);
        tx.open_file(options, move |s: &mut Ui, result, tx| {
            if let Some(path) = s.dialog_path(result, tx) {
                s.warps.choose = None;
                s.warp_command(WarpCommand::ImportSrs { path, uid, profile: None });
            }
        });
    }

    pub fn choose_profile(&mut self, key: Option<String>) {
        let Some(pick) = self.warps.choose.take() else {
            return;
        };
        if let Some(key) = key {
            self.warp_command(WarpCommand::ImportSrs {
                path: pick.path,
                uid: pick.uid,
                profile: Some(key),
            });
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

    /// Starts loading the background image the settings name, if it isn't loaded or on its way
    pub fn load_background(&mut self) {
        let path = self.settings.background_image.clone();
        if path.is_empty() {
            self.background = None;
            self.background_loading = None;
            self.background_error = None;
            return;
        }
        if self.background.as_ref().is_some_and(|(loaded, _)| *loaded == path) || self.background_loading.as_ref() == Some(&path) {
            return;
        }
        let (Some(images), Some(loader)) = (&self.images, &self.background_loader) else {
            return;
        };
        self.background_error = None;
        if background::is_raster(&path) {
            self.background_loading = Some(path.clone());
            loader.request(path, |path| {
                let prepared = background::prepare(&path);
                (path, prepared)
            });
        } else {
            // Rune draws vector images at the size they're shown at, which only needs the size
            let image = images.file(&path, 1024.0, 1.0);
            self.background_edges = None;
            self.background = Some((path, image));
        }
    }

    /// Loads the background on this thread and waits for it, for snapshots, which draw a single frame
    pub fn load_background_now(&mut self) {
        let path = self.settings.background_image.clone();
        let Some(images) = &self.images else {
            return;
        };
        if path.is_empty() {
            return;
        }
        let image = if background::is_raster(&path) {
            match background::prepare(&path) {
                Ok(prepared) => {
                    self.background_edges = Some(prepared.edges);
                    images.rgba(prepared.width, prepared.height, prepared.rgba)
                }
                Err(e) => {
                    self.background_error = Some(e);
                    return;
                }
            }
        } else {
            self.background_edges = None;
            images.file(&path, 1024.0, 1.0)
        };
        let _ = image.wait();
        self.background = Some((path, image));
    }

    /// Takes a background the loader finished. True when that changed anything
    pub fn background_ready(&mut self) -> bool {
        let Some((path, prepared)) = self.background_loader.as_ref().and_then(|loader| loader.take()) else {
            return false;
        };
        if self.background_loading.as_ref() == Some(&path) {
            self.background_loading = None;
        }
        // The user picked another image meanwhile
        if path != self.settings.background_image {
            return false;
        }
        match prepared {
            Ok(prepared) => {
                if let Some(images) = &self.images {
                    self.background_edges = Some(prepared.edges);
                    self.background = Some((path, images.rgba(prepared.width, prepared.height, prepared.rgba)));
                }
            }
            Err(e) => {
                tracing::warn!(%e, "couldn't load the background");
                self.background = None;
                self.background_error = Some(e);
            }
        }
        true
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
        self.load_background();
    }

    pub fn set_image_fit(&mut self, fit: ImageFit) {
        self.settings_mut(|s| s.image_fit = fit);
    }

    /// Sets the point of the background image that matters. Whole percents, so dragging doesn't
    /// save a new file for every pixel moved
    pub fn set_background_anchor(&mut self, anchor: [f32; 2]) {
        let anchor = anchor.map(whole_percent);
        if anchor != self.settings.background_anchor {
            self.settings_mut(|s| s.background_anchor = anchor);
        }
    }

    /// Sets where in the window the image's point goes, in whole percents as the point is
    pub fn set_background_target(&mut self, target: Option<[f32; 2]>) {
        let target = target.map(|target| target.map(whole_percent));
        if target != self.settings.background_target {
            self.settings_mut(|s| s.background_target = target);
        }
    }

    pub fn set_background_fill(&mut self, fill: BackgroundFill) {
        self.settings_mut(|s| s.background_fill = fill);
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

    const AT_TOP: PageScroll = PageScroll {
        at_top: true,
        at_bottom: false,
    };
    const AT_BOTTOM: PageScroll = PageScroll {
        at_top: false,
        at_bottom: true,
    };
    const BOTH: PageScroll = PageScroll {
        at_top: true,
        at_bottom: true,
    };

    #[test]
    fn a_wheel_notch_past_the_banner_turns_to_the_history_and_back() {
        let mut s = ui();
        assert!(snap_wheel(&mut s, -100.0, true, false, BOTH, 1.0));
        assert_eq!(s.warps.page, WarpPage::History);
        // The rest of the gesture is held, it doesn't turn the page straight back
        assert!(snap_wheel(&mut s, 100.0, true, false, BOTH, 1.2));
        assert_eq!(s.warps.page, WarpPage::History);
        assert!(snap_wheel(&mut s, 100.0, true, false, BOTH, 2.0));
        assert_eq!(s.warps.page, WarpPage::Banner);
    }

    #[test]
    fn a_page_scrolls_itself_until_its_end() {
        let mut s = ui();
        // The banner taller than its room, partway down: it scrolls
        let partway = PageScroll {
            at_top: false,
            at_bottom: false,
        };
        assert!(!snap_wheel(&mut s, -100.0, true, false, partway, 1.0));
        assert_eq!(s.warps.page, WarpPage::Banner);
        // Scrolling up at the banner's top goes nowhere either
        assert!(!snap_wheel(&mut s, 100.0, true, false, AT_TOP, 2.0));

        s.warps.page = WarpPage::History;
        assert!(!snap_wheel(&mut s, -100.0, true, false, AT_TOP, 3.0), "the history scrolls down");
        assert!(!snap_wheel(&mut s, 100.0, true, false, AT_BOTTOM, 4.0), "and back up");
        assert_eq!(s.warps.page, WarpPage::History);
    }

    #[test]
    fn a_trackpad_turns_the_page_once_it_pulls_far_enough() {
        let mut s = ui();
        for i in 0..5 {
            assert!(snap_wheel(&mut s, -10.0, false, false, AT_BOTTOM, 1.0 + i as f64 * 0.016));
        }
        assert_eq!(s.warps.page, WarpPage::Banner, "50 points isn't enough");
        // A pause starts the pull over
        assert!(snap_wheel(&mut s, -10.0, false, false, AT_BOTTOM, 2.0));
        assert_eq!(s.warps.page, WarpPage::Banner);
        for i in 1..6 {
            snap_wheel(&mut s, -10.0, false, false, AT_BOTTOM, 2.0 + i as f64 * 0.016);
        }
        assert_eq!(s.warps.page, WarpPage::History);
        // Coasting after the fingers lift never turns it
        let mut s = ui();
        for i in 0..20 {
            snap_wheel(&mut s, -30.0, false, true, AT_BOTTOM, 1.0 + i as f64 * 0.016);
        }
        assert_eq!(s.warps.page, WarpPage::Banner);
    }

    fn ui() -> Ui {
        Ui::new(Settings::default())
    }

    #[test]
    fn the_latest_login_is_selected_and_its_data_starts_over() {
        let mut ui = ui();
        ui.account_logged_in(1);
        ui.inventory_changed(1, Inventory {
            relics: 3,
            ..Default::default()
        });
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
            funds: Funds::default(),
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
    fn a_picked_up_account_is_flagged_until_it_logs_in_again() {
        let mut ui = ui();
        let picked_up = |ui: &Ui| ui.accounts.iter().find(|a| a.uid == 7).map(|a| a.picked_up);

        // The session's events can come before the account's own
        assert!(!ui.session_event(&SessionEvent::AccountSeen { uid: 7 }));
        assert!(ui.session_event(&SessionEvent::PickedUp { uid: 7 }));
        ui.account_logged_in(7);
        assert_eq!(picked_up(&ui), Some(true));

        assert!(ui.session_event(&SessionEvent::AccountSeen { uid: 7 }));
        assert_eq!(picked_up(&ui), Some(false));
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
        // Older files have no theme or accent, they get the defaults
        assert_eq!(settings.theme, super::super::settings::ThemeMode::System);
        assert_eq!(settings.accent, super::super::settings::Accent::Gold);
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

/// 0 to 1, to the nearest whole percent
fn whole_percent(v: f32) -> f32 {
    (v.clamp(0.0, 1.0) * 100.0).round() / 100.0
}
