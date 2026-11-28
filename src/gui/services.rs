//! The window's way to the rest of the app: the account manager, the WebSocket server, the settings
//! file and the updater.
//!
//! Each call starts its work on the tokio runtime and returns at once. When the work is done its
//! result is handed back to the UI thread through the [`AppHandle`], so the window never waits.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use futures::lock::Mutex;
use reliquary_archiver::export::Exporter;
use reliquary_archiver::export::fribbels::Export;
use rune::{AppHandle, FileDialogOptions, Tx};
use tokio::runtime::Handle;
use tokio::sync::watch;
use tracing::{debug, error, info, warn};

use super::settings::{Settings, SettingsFile};
use super::state::Ui;
use crate::accounts::MultiAccountManager;
use crate::capture::{Replay, Tap};
use crate::warps::service::WarpService;
use crate::websocket::PortCommand;

pub struct Services {
    rt: Handle,
    app: AppHandle<Ui>,
    manager: Arc<Mutex<MultiAccountManager>>,
    selected_tx: watch::Sender<Option<u32>>,
    port_tx: watch::Sender<PortCommand>,
    settings_file: SettingsFile,
    /// Updates were turned off from the command line
    no_update: bool,
    /// Kept for as long as the app runs, dropping it removes the icon
    _tray: Option<rune::Tray>,
}

/// What [`Services::start`] starts from, besides the window
pub struct Start {
    pub rt: Handle,
    /// Where packets are captured from
    pub tap: Tap,
    pub settings_file: SettingsFile,
    /// The port the WebSocket server opens first
    pub ws_port: u16,
    /// The system tray icon, kept for as long as the app runs
    pub tray: Option<rune::Tray>,
    /// Capturing needs administrator rights the app doesn't have, so it won't start
    pub needs_admin: bool,
    /// Updates were turned off from the command line
    pub no_update: bool,
}

/// The Save dialog for an export
pub fn export_dialog() -> FileDialogOptions {
    FileDialogOptions {
        title: "Save export".into(),
        file_name: Some(
            jiff::ZonedDateTime::now()
                .strftime("archive_output-%Y-%m-%dT%H-%M-%S.json")
                .to_string(),
        ),
        ..Default::default()
    }
    .filter("JSON files", &["json"])
}

/// The Open dialog for a capture file
pub fn capture_file_dialog() -> FileDialogOptions {
    FileDialogOptions {
        title: "Open capture file".into(),
        ..Default::default()
    }
    .filter("Packet captures", &Replay::file_extensions())
}

impl Services {
    /// Starts capturing, the WebSocket server and everything that feeds the window
    pub fn start(app: AppHandle<Ui>, start: Start) -> Services {
        let Start {
            rt,
            tap,
            settings_file,
            ws_port,
            tray,
            needs_admin,
            no_update,
        } = start;
        let manager = Arc::new(Mutex::new(MultiAccountManager::new()));
        let (selected_tx, selected_rx) = watch::channel(None);
        // Nothing shows the warp history yet, what it reports is only logged
        let warps = WarpService::start(crate::warps::store::default_path(), |update| debug!(?update, "warps"));
        let (port_tx, port_rx) = watch::channel(PortCommand::Open(ws_port));

        rt.spawn(settings_file.writer());
        rt.spawn(super::forward_server(app.clone(), manager.clone(), port_rx, selected_rx));
        if needs_admin {
            warn!("not capturing, administrator rights are needed");
        } else {
            rt.spawn(super::forward_worker(app.clone(), manager.clone(), tap, warps));
        }
        rt.spawn(super::forward_logs(app.clone()));

        Services {
            rt,
            app,
            manager,
            selected_tx,
            port_tx,
            settings_file,
            no_update: no_update || std::env::var("NO_SELF_UPDATE").is_ok_and(|v| v == "1"),
            _tray: tray,
        }
    }

    pub fn save_settings(&self, settings: &Settings) {
        self.settings_file.save_if_changed(settings);
    }

    /// Saves the settings and writes them at once, for when the app is about to exit
    pub fn flush_settings(&self, settings: &Settings) {
        self.save_settings(settings);
        self.settings_file.flush();
    }

    pub fn select_account(&self, uid: u32) {
        self.selected_tx.send_replace(Some(uid));
    }

    pub fn open_port(&self, port: u16) {
        self.port_tx.send_replace(PortCommand::Open(port));
    }

    pub fn close_port(&self) {
        self.port_tx.send_replace(PortCommand::Close);
    }

    /// Exports the account's data as it is now to `path`
    pub fn save_export(&self, uid: u32, path: PathBuf) {
        let (manager, app) = (self.manager.clone(), self.app.clone());
        self.rt.spawn(async move {
            let export = manager.lock().await.account_exporter(uid).and_then(|exporter| exporter.export());
            match export {
                Some(export) => write_export(app, export, path).await,
                None => app.update(|s, tx| s.notify_error("Couldn't export", "The account's data isn't complete yet", tx)),
            }
        });
    }

    /// Writes an export that was read from a capture file
    pub fn save_file_export(&self, export: Export, path: PathBuf) {
        self.rt.spawn(write_export(self.app.clone(), export, path));
    }

    /// Reads a capture file, then asks where to save what was in it
    pub fn read_capture_file(&self, path: PathBuf) {
        let app = self.app.clone();
        let Some(replay) = Replay::for_path(path.clone()) else {
            let name = file_name(&path);
            app.update(move |s, tx| {
                s.reading_file = false;
                s.notify_error(
                    "Can't read that file",
                    format!("{name} isn't a capture file this version can read"),
                    tx,
                );
            });
            return;
        };
        self.rt.spawn(async move {
            let export = crate::session::export_from_file(replay).await;
            app.update(move |s, tx| s.capture_file_read(export.map(Box::new), tx));
        });
    }

    pub fn save_logs(&self, path: PathBuf) {
        let app = self.app.clone();
        self.rt.spawn_blocking(move || {
            let text = crate::logging::lines().join("\n");
            let result = std::fs::write(&path, text);
            app.update(move |s, tx| match result {
                Ok(()) => s.notify_saved("Saved logs", path, tx),
                Err(e) => s.notify_error("Couldn't save logs", e.to_string(), tx),
            });
        });
    }

    pub fn can_update(&self) -> bool {
        cfg!(windows) && !self.no_update
    }

    pub fn check_for_update(&self) {
        #[cfg(windows)]
        {
            let app = self.app.clone();
            self.rt.spawn_blocking(move || {
                let result = crate::update::check_for_update().map_err(|e| e.to_string());
                match &result {
                    Ok(Some(version)) => info!(version, "update available"),
                    Ok(None) => info!("already up-to-date"),
                    Err(e) => error!(%e, "failed to check for updates"),
                }
                app.update(move |s, _| s.update_checked(result));
            });
        }
    }

    pub fn install_update(&self) {
        #[cfg(windows)]
        {
            let app = self.app.clone();
            self.rt.spawn_blocking(move || {
                let error = match crate::update::update_noninteractive() {
                    Ok(true) => {
                        app.update(|_, tx| {
                            crate::update::request_spawn_after_exit();
                            tx.quit();
                        });
                        return;
                    }
                    Ok(false) => "The update reported no changes".to_string(),
                    Err(e) => e.to_string(),
                };
                error!(%error, "failed to update");
                app.update(move |s, _| s.update_failed(error));
            });
        }
    }

    /// Adds or removes the app from the programs Windows starts on login
    pub fn set_run_on_start(&self, enabled: bool) -> Result<(), String> {
        #[cfg(windows)]
        return super::run_on_start::set_run_on_start(enabled).map_err(|e| e.to_string());
        #[cfg(not(windows))]
        {
            let _ = enabled;
            Err("Only available on Windows".into())
        }
    }

    /// Starts the app again with administrator rights and quits this instance
    pub fn restart_as_admin(&self, tx: &mut Tx) {
        #[cfg(pktmon_only)]
        match crate::elevation::escalate_to_admin() {
            Ok(()) => tx.quit(),
            Err(e) => error!(%e, "failed to restart as administrator"),
        }
        #[cfg(not(pktmon_only))]
        let _ = tx;
    }
}

/// Opens the folder holding `path`
pub fn reveal(path: &Path) {
    if let Some(folder) = path.parent()
        && let Err(e) = open::that(folder)
    {
        error!(%e, "failed to open folder");
    }
}

/// The file's name, for a notice
pub fn file_name(path: &Path) -> String {
    path.file_name()
        .map_or_else(|| path.display().to_string(), |name| name.to_string_lossy().into_owned())
}

async fn write_export(app: AppHandle<Ui>, export: Export, path: PathBuf) {
    let result = tokio::task::spawn_blocking({
        let path = path.clone();
        move || -> Result<(), String> {
            let json = serde_json::to_string_pretty(&export).map_err(|e| e.to_string())?;
            std::fs::write(&path, json).map_err(|e| e.to_string())
        }
    })
    .await
    .unwrap_or_else(|e| Err(e.to_string()));

    match &result {
        Ok(()) => info!("Saved export to {}", path.display()),
        Err(e) => error!(%e, "Failed to save export to {}", path.display()),
    }
    app.update(move |s, tx| match result {
        Ok(()) => s.notify_saved("Export saved", path, tx),
        Err(e) => s.notify_error("Couldn't save the export", e, tx),
    });
}
