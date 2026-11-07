//! User settings and their persistence.
//!
//! Settings are stored as JSON in the user's local app data folder. They are loaded once before the
//! GUI starts, and [`rgui::update`](crate::rgui::update) saves them whenever a message leaves them
//! different from what was saved last, so code that changes settings doesn't have to save them.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use tokio::sync::watch;

use crate::rgui::run_on_start::registry_matches_settings;

/// Background image scaling mode.
///
/// Controls how a custom background image is displayed within the window.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize, Default)]
pub enum ImageFit {
    /// Stretch to fill the container
    Fill,
    /// Scale to fit inside the container while maintaining aspect ratio
    Contain,
    /// Scale to cover the container while maintaining aspect ratio (may crop, default)
    #[default]
    Cover,
    /// Like contain but never scale up beyond intrinsic size
    ScaleDown,
    /// Display at intrinsic size with no scaling
    None,
}

/// User-configurable application settings.
///
/// These settings are persisted to disk as JSON in the user's app data folder.
/// Settings are saved automatically whenever they change, see [`SettingsFile`].
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Path to a custom background image (empty string = no image)
    pub background_image: String,
    /// How the background image is scaled/positioned
    pub image_fit: ImageFit,
    /// Opacity of the background image (0.0 - 1.0)
    pub background_opacity: f32,
    /// Whether to render text shadows for better readability over images
    pub text_shadow_enabled: bool,
    /// Automatically apply updates without prompting
    pub always_update: bool,
    /// Hide to system tray instead of closing when clicking X
    pub minimize_to_tray_on_close: bool,
    /// Hide to system tray instead of taskbar when minimizing
    pub minimize_to_tray_on_minimize: bool,
    /// Launch the application on Windows startup
    pub run_on_start: bool,
    /// Start the application minimized/hidden
    pub start_minimized: bool,
    /// Port for the WebSocket server (0 = auto-assign)
    pub ws_port: u16,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            background_image: "".to_string(),
            image_fit: ImageFit::default(),
            background_opacity: 0.12,
            text_shadow_enabled: false,
            always_update: false,
            minimize_to_tray_on_close: false,
            minimize_to_tray_on_minimize: false,
            run_on_start: false,
            start_minimized: false,
            ws_port: 23313,
        }
    }
}

/// Returns the path to the settings file within the app data directory.
fn settings_path(appdata: PathBuf) -> PathBuf {
    appdata.join("reliquary-archiver").join("settings.json")
}

/// The settings file, and what it holds.
///
/// A single writer task, [`SettingsFile::writer`], writes the settings whenever they change. Changes
/// that come in while it is writing are combined, and the latest settings always win. Whatever it
/// hasn't written yet when the app exits is written when the `SettingsFile` is dropped.
pub struct SettingsFile {
    path: Option<PathBuf>,
    /// The settings that were last handed to the writer
    to_save: watch::Sender<Settings>,
    /// Never read, so the writer's clone of it sees every change made since the file was loaded
    changes: watch::Receiver<Settings>,
    /// What the file holds. Locked while writing, so only one write happens at a time
    written: Arc<Mutex<Settings>>,
}

impl SettingsFile {
    /// `saved` is what the file at `path` holds
    fn new(path: Option<PathBuf>, saved: Settings) -> Self {
        let written = Arc::new(Mutex::new(saved.clone()));
        let (to_save, changes) = watch::channel(saved);
        Self {
            path,
            to_save,
            changes,
            written,
        }
    }

    /// Saves the settings if they changed since they were last saved
    pub fn save_if_changed(&self, settings: &Settings) {
        self.to_save.send_if_modified(|to_save| {
            if to_save == settings {
                return false;
            }

            *to_save = settings.clone();
            true
        });
    }

    /// Writes the settings whenever they change, for as long as the app runs
    pub fn writer(&self) -> impl Future<Output = ()> + Send + 'static {
        let path = self.path.clone();
        let mut changes = self.changes.clone();
        let written = self.written.clone();

        async move {
            let Some(path) = path else {
                return;
            };

            while changes.changed().await.is_ok() {
                changes.borrow_and_update();

                let (path, written, latest) = (path.clone(), written.clone(), changes.clone());
                // The latest settings are read once it is this write's turn, so the newest ones end up on disk
                tokio::task::spawn_blocking(move || flush(&path, &written, || latest.borrow().clone()))
                    .await
                    .ok();
            }
        }
    }
}

impl Drop for SettingsFile {
    /// The writer task doesn't get to run anymore once the app exits, so write what it hasn't gotten to
    fn drop(&mut self) {
        if let Some(path) = &self.path {
            flush(path, &self.written, || self.to_save.borrow().clone());
        }
    }
}

/// Writes the latest settings, unless the file already holds them
fn flush(path: &Path, written: &Mutex<Settings>, latest: impl FnOnce() -> Settings) {
    let mut written = written.lock().unwrap_or_else(PoisonError::into_inner);
    let latest = latest();
    if *written == latest {
        return;
    }

    match write(path, &latest) {
        Ok(()) => *written = latest,
        Err(e) => tracing::error!("Failed to save settings to {}: {}", path.display(), e),
    }
}

/// Writes the settings to the file at `path`.
///
/// They are written to a temporary file first, which then replaces the settings file, so the
/// settings file is never left half written if the application crashes or is killed meanwhile.
fn write(path: &Path, settings: &Settings) -> io::Result<()> {
    // The process id keeps two running instances from writing to the same temporary file
    let temp_path = path.with_extension(format!("json.{}.tmp", std::process::id()));

    std::fs::create_dir_all(path.parent().unwrap())?;
    std::fs::write(&temp_path, serde_json::to_string_pretty(settings)?)?;
    std::fs::rename(&temp_path, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&temp_path);
    })
}

/// Loads settings synchronously from disk before the GUI initializes.
///
/// Uses the new `raxis::get_local_app_data()` synchronous helper to resolve the
/// settings path without needing an async task, preventing race conditions between
/// settings loading and other startup tasks (update checks, WebSocket port binding).
///
/// Returns the settings to use and the settings file. Without a usable settings file, the
/// defaults are used and nothing is written until a setting is changed.
pub fn load() -> (Settings, SettingsFile) {
    // For when saving could destroy settings the file still holds
    let unsaved_defaults = || (Settings::default(), SettingsFile::new(None, Settings::default()));

    let Some(appdata) = raxis::get_local_app_data() else {
        tracing::warn!("Could not determine local app data path, using default settings that won't be saved");
        return unsaved_defaults();
    };

    let path = settings_path(appdata);
    let defaults = || (Settings::default(), SettingsFile::new(Some(path.clone()), Settings::default()));

    let content = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            tracing::info!("No settings file found at {}, using defaults", path.display());
            return defaults();
        }
        Err(e) => {
            tracing::error!("Failed to read settings file, using default settings that won't be saved: {}", e);
            return unsaved_defaults();
        }
    };

    let mut settings: Settings = match serde_json::from_str(&content) {
        Ok(s) => s,
        Err(e) => {
            // The first change would overwrite the file, keep a copy so the settings aren't lost
            let backup = path.with_extension("json.bak");
            return match std::fs::rename(&path, &backup) {
                Ok(()) => {
                    tracing::error!(
                        "Failed to parse settings file, moved it to {} and using defaults: {}",
                        backup.display(),
                        e
                    );
                    defaults()
                }
                Err(backup_error) => {
                    tracing::error!(
                        "Failed to parse settings file, using default settings that won't be saved: {} (could not back it up: {})",
                        e,
                        backup_error
                    );
                    unsaved_defaults()
                }
            };
        }
    };
    let file = SettingsFile::new(Some(path.clone()), settings.clone());

    // Reconcile run_on_start setting with registry state.
    // e.g. user moves the exe after enabling/disabling run on start — in case
    // of mismatch, update settings in memory. They differ from the file now, so
    // the first update saves the correction.
    if let Ok(false) = registry_matches_settings(settings.run_on_start) {
        settings.run_on_start = !settings.run_on_start;
    }

    tracing::info!("Loaded settings from {}", path.display());
    (settings, file)
}
