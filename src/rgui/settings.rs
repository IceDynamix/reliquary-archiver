//! User settings and their persistence.
//!
//! Settings are stored as JSON in the user's local app data folder. They are loaded once before the
//! GUI starts, and [`rgui::update`](crate::rgui::update) saves them whenever a message leaves them
//! different from what was saved last, so code that changes settings doesn't have to save them.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use raxis::runtime::task::Task;

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

/// Counter for generating unique temp file names within this process.
static SAVE_COUNTER: AtomicU64 = AtomicU64::new(0);

/// The settings file, and what it holds.
pub struct SettingsFile {
    /// The settings that were last saved
    saved: Settings,
}

impl SettingsFile {
    /// `saved` is what the file holds
    fn new(saved: Settings) -> Self {
        Self { saved }
    }

    /// Saves the settings if they changed since they were last saved
    pub fn save_if_changed<M: Send + 'static>(&mut self, settings: &Settings) -> Option<Task<M>> {
        if self.saved == *settings {
            return None;
        }

        self.saved = settings.clone();
        save(settings)
    }
}

/// Persists the current settings to disk asynchronously.
///
/// Settings are saved atomically to the user's local app data folder as JSON.
/// We write to a temporary file first, then rename it to avoid corruption
/// if the application crashes or is killed during the write.
///
/// Each save uses a unique temp filename (process ID + counter) to prevent
/// race conditions if multiple saves happen concurrently.
fn save<M: Send + 'static>(settings: &Settings) -> Option<Task<M>> {
    let settings = settings.clone();
    // Generate unique temp filename to avoid races between concurrent saves
    let unique_id = SAVE_COUNTER.fetch_add(1, Ordering::Relaxed);
    let pid = std::process::id();

    let Some(appdata) = raxis::get_local_app_data() else {
        tracing::error!("Could not determine local app data path, settings not saved");
        return None;
    };

    let path = settings_path(appdata);

    Some(
        Task::future(async move {
            let temp_path = path.with_extension(format!("json.{}.{}.tmp", pid, unique_id));

            if let Err(e) = tokio::fs::create_dir_all(path.parent().unwrap()).await {
                tracing::error!("Failed to create settings directory: {}", e);
                return;
            }

            let json = match serde_json::to_string_pretty(&settings) {
                Ok(json) => json,
                Err(e) => {
                    tracing::error!("Failed to serialize settings: {}", e);
                    return;
                }
            };

            // Write to temporary file first (unique per save operation)
            if let Err(e) = tokio::fs::write(&temp_path, &json).await {
                tracing::error!("Failed to write settings to temp file: {}", e);
                return;
            }

            // Atomically rename temp file to actual settings file
            if let Err(e) = tokio::fs::rename(&temp_path, &path).await {
                tracing::error!("Failed to rename temp settings file: {}", e);
                let _ = tokio::fs::remove_file(&temp_path).await;
            }
        })
        .discard(),
    )
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
    let defaults = || (Settings::default(), SettingsFile::new(Settings::default()));

    let Some(appdata) = raxis::get_local_app_data() else {
        tracing::warn!("Could not determine local app data path, using default settings");
        return defaults();
    };

    let path = settings_path(appdata);

    let content = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            tracing::info!("No settings file found at {}, using defaults", path.display());
            return defaults();
        }
        Err(e) => {
            tracing::error!("Failed to read settings file: {}", e);
            return defaults();
        }
    };

    let mut settings: Settings = match serde_json::from_str(&content) {
        Ok(s) => s,
        Err(e) => {
            tracing::error!("Failed to parse settings file: {}", e);
            return defaults();
        }
    };
    let file = SettingsFile::new(settings.clone());

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
