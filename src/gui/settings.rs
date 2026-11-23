//! User settings and their persistence.
//!
//! Settings are stored as JSON in the user's local app data folder (the config folder off Windows).
//! They are loaded once before the GUI starts, and saved by [`Ui::settings_mut`](super::state::Ui::settings_mut)
//! whenever a change leaves them different from what was saved last.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use tokio::sync::watch;

/// How a custom background image fills the window
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum ImageFit {
    /// Stretched to the window's shape
    Fill,
    /// As large as fits inside the window, keeping its shape
    Contain,
    /// Covers the whole window, keeping its shape and cropping what doesn't fit
    #[default]
    Cover,
    /// Like contain, but never larger than the image itself
    ScaleDown,
    /// At the image's own size, centred
    None,
}

/// What fills the window where the background image doesn't reach, when it's placed smaller than
/// the window (see [`ImageFit`])
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum BackgroundFill {
    /// The window's own background
    None,
    /// The image again, covering the window behind, blurred
    #[default]
    Blur,
    /// The average colour along each of the image's edges
    EdgeColor,
    /// The image's outermost pixels drawn out to the window's edges
    Extend,
    /// The image reflected at its edges, and again past those
    Mirror,
    /// Reflected as `Mirror`, blurring more the further from the image it reaches
    MirrorBlur,
    /// The image again side by side
    Repeat,
}

impl BackgroundFill {
    pub const ALL: [BackgroundFill; 7] = [
        BackgroundFill::None,
        BackgroundFill::Blur,
        BackgroundFill::EdgeColor,
        BackgroundFill::Extend,
        BackgroundFill::Mirror,
        BackgroundFill::MirrorBlur,
        BackgroundFill::Repeat,
    ];

    pub fn label(self) -> &'static str {
        match self {
            BackgroundFill::None => "None",
            BackgroundFill::Blur => "Blur",
            BackgroundFill::EdgeColor => "Edge colour",
            BackgroundFill::Extend => "Extend",
            BackgroundFill::Mirror => "Mirror",
            BackgroundFill::MirrorBlur => "Mirror, blurred",
            BackgroundFill::Repeat => "Repeat",
        }
    }
}

/// Light or dark
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum ThemeMode {
    /// Whatever the system uses
    #[default]
    System,
    Dark,
    Light,
}

/// The colour that marks what matters, named after the gems relics are set with
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum Accent {
    #[default]
    Gold,
    Silver,
    Ruby,
    Amethyst,
    Emerald,
    Sapphire,
}

impl Accent {
    pub const ALL: [Accent; 6] = [
        Accent::Gold,
        Accent::Silver,
        Accent::Ruby,
        Accent::Amethyst,
        Accent::Emerald,
        Accent::Sapphire,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Accent::Gold => "Gold",
            Accent::Silver => "Silver",
            Accent::Ruby => "Ruby",
            Accent::Amethyst => "Amethyst",
            Accent::Emerald => "Emerald",
            Accent::Sapphire => "Sapphire",
        }
    }
}

/// Where the cards sit in a window that's bigger than they are
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum Anchor {
    TopLeft,
    #[default]
    Top,
    TopRight,
    Left,
    Center,
    Right,
    BottomLeft,
    Bottom,
    BottomRight,
}

impl Anchor {
    /// Row by row, from the top left
    pub const ALL: [Anchor; 9] = [
        Anchor::TopLeft,
        Anchor::Top,
        Anchor::TopRight,
        Anchor::Left,
        Anchor::Center,
        Anchor::Right,
        Anchor::BottomLeft,
        Anchor::Bottom,
        Anchor::BottomRight,
    ];

    /// The column and row of the 3 by 3 grid this is in, each 0, 1 or 2
    pub fn cell(self) -> (usize, usize) {
        let index = Anchor::ALL.iter().position(|a| *a == self).unwrap_or(1);
        (index % 3, index / 3)
    }

    pub fn label(self) -> &'static str {
        match self {
            Anchor::TopLeft => "Top left",
            Anchor::Top => "Top",
            Anchor::TopRight => "Top right",
            Anchor::Left => "Left",
            Anchor::Center => "Center",
            Anchor::Right => "Right",
            Anchor::BottomLeft => "Bottom left",
            Anchor::Bottom => "Bottom",
            Anchor::BottomRight => "Bottom right",
        }
    }
}

/// User-configurable application settings.
///
/// These settings are persisted to disk as JSON in the user's app data folder.
/// Settings are saved automatically whenever they change, see [`SettingsFile`].
/// Fields that older versions wrote and that are gone now are ignored when loading.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Path to an image drawn behind the window's content, empty for none
    pub background_image: String,
    pub image_fit: ImageFit,
    /// How strongly the background image shows, from 0 to 1
    pub background_opacity: f32,
    /// The point of the background image that matters, from 0 to 1 across and down it
    pub background_anchor: [f32; 2],
    /// Where in the window that point goes, from 0 to 1 across and down it: as near as the image
    /// allows without leaving a gap it could cover, or sliding out of view. None puts it at the
    /// same place in the window as it is in the image (CSS's `object-position`)
    pub background_target: Option<[f32; 2]>,
    /// What fills the window where the image doesn't reach
    pub background_fill: BackgroundFill,
    /// Where the cards sit when the window is bigger than they are
    pub card_anchor: Anchor,
    pub theme: ThemeMode,
    pub accent: Accent,
    /// Install updates without asking
    pub always_update: bool,
    /// Closing the window leaves the app running in the system tray
    #[serde(alias = "minimize_to_tray_on_close")]
    pub close_to_tray: bool,
    /// Launch the application on Windows startup
    pub run_on_start: bool,
    /// Start without showing the window
    pub start_minimized: bool,
    /// Port for the WebSocket server (0 = auto-assign)
    pub ws_port: u16,
    /// Where the window was when it last closed, to open it there again
    pub window: Option<WindowFrame>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            background_image: String::new(),
            image_fit: ImageFit::default(),
            background_opacity: 0.12,
            background_anchor: [0.5, 0.5],
            background_target: Some([0.5, 0.5]),
            background_fill: BackgroundFill::default(),
            card_anchor: Anchor::default(),
            theme: ThemeMode::default(),
            accent: Accent::default(),
            always_update: false,
            close_to_tray: false,
            run_on_start: false,
            start_minimized: false,
            ws_port: 23313,
            window: None,
        }
    }
}

/// Where the window sits on the desktop, in physical pixels
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct WindowFrame {
    /// The frame's top-left
    pub position: (i32, i32),
    /// The content's size
    pub size: (u32, u32),
    /// Opens maximized, and goes back to `position` and `size` when restored
    pub maximized: bool,
}

impl WindowFrame {
    /// The frame to open at next time for a window at `placement`. While it's maximized or
    /// minimized that's the frame restoring it returns to, which only Windows keeps a record of;
    /// elsewhere the `last` frame stands in for it. Full screen keeps the last frame.
    pub fn of(placement: &rune::WindowPlacement, last: Option<WindowFrame>) -> Option<WindowFrame> {
        if placement.full_screen {
            return last;
        }
        let (position, size) = if placement.maximized || placement.minimized {
            match (placement.restore, last) {
                (Some(restore), _) => restore,
                (None, Some(last)) => (last.position, last.size),
                (None, None) => return None,
            }
        } else {
            (placement.position, placement.content_size)
        };
        Some(WindowFrame {
            position,
            size,
            // A minimized window opens as it was before it was minimized, as far as that's known
            maximized: if placement.minimized {
                last.is_some_and(|last| last.maximized)
            } else {
                placement.maximized
            },
        })
    }
}

/// The app's own folder, which the settings file and the scaled backgrounds go in: in the local
/// app data folder on Windows, in the user's config folder elsewhere
pub fn data_dir() -> Option<PathBuf> {
    let base = if cfg!(windows) {
        std::env::var_os("LOCALAPPDATA").map(PathBuf::from)
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
    };
    base.filter(|path| path.is_absolute()).map(|path| path.join("reliquary-archiver"))
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

impl SettingsFile {
    /// Writes what the writer task hasn't gotten to yet, for when the app is about to exit
    pub fn flush(&self) {
        if let Some(path) = &self.path {
            flush(path, &self.written, || self.to_save.borrow().clone());
        }
    }
}

impl Drop for SettingsFile {
    /// The writer task doesn't get to run anymore once the app exits, so write what it hasn't gotten to
    fn drop(&mut self) {
        self.flush();
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

/// Loads settings synchronously from disk before the GUI initializes, so the update check and
/// the WebSocket server start with the user's settings.
///
/// Returns the settings to use and the settings file. Without a usable settings file, the
/// defaults are used and nothing is written until a setting is changed.
pub fn load() -> (Settings, SettingsFile) {
    // For when saving could destroy settings the file still holds
    let unsaved_defaults = || (Settings::default(), SettingsFile::new(None, Settings::default()));

    let Some(dir) = data_dir() else {
        tracing::warn!("Could not determine where to keep settings, using default settings that won't be saved");
        return unsaved_defaults();
    };

    let path = dir.join("settings.json");
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

    #[cfg_attr(not(windows), allow(unused_mut))]
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
    #[cfg(windows)]
    if let Ok(false) = super::run_on_start::registry_matches_settings(settings.run_on_start) {
        settings.run_on_start = !settings.run_on_start;
    }

    tracing::info!("Loaded settings from {}", path.display());
    (settings, file)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn placement() -> rune::WindowPlacement {
        rune::WindowPlacement {
            position: (100, 50),
            content_size: (920, 1400),
            scale: 2.0,
            ..Default::default()
        }
    }

    #[test]
    fn a_window_reopens_where_it_was() {
        let frame = WindowFrame::of(&placement(), None).unwrap();
        assert_eq!((frame.position, frame.size, frame.maximized), ((100, 50), (920, 1400), false));
    }

    #[test]
    fn a_maximized_window_keeps_the_frame_it_restores_to() {
        let maximized = rune::WindowPlacement {
            position: (-8, -8),
            content_size: (2560, 1400),
            maximized: true,
            restore: Some(((300, 200), (900, 1300))),
            ..placement()
        };
        let frame = WindowFrame::of(&maximized, None).unwrap();
        assert_eq!((frame.position, frame.size, frame.maximized), ((300, 200), (900, 1300), true));

        // Without a record of it, the last frame stands in
        let last = WindowFrame {
            position: (1, 2),
            size: (3, 4),
            maximized: false,
        };
        let frame = WindowFrame::of(
            &rune::WindowPlacement {
                restore: None,
                ..maximized
            },
            Some(last),
        )
        .unwrap();
        assert_eq!((frame.position, frame.size, frame.maximized), ((1, 2), (3, 4), true));
        assert_eq!(
            WindowFrame::of(
                &rune::WindowPlacement {
                    restore: None,
                    ..maximized
                },
                None
            ),
            None
        );
    }

    #[test]
    fn a_minimized_window_reopens_as_it_was_before() {
        let last = WindowFrame {
            position: (1, 2),
            size: (3, 4),
            maximized: true,
        };
        let minimized = rune::WindowPlacement {
            minimized: true,
            restore: Some(((300, 200), (900, 1300))),
            ..placement()
        };
        let frame = WindowFrame::of(&minimized, Some(last)).unwrap();
        assert_eq!((frame.position, frame.size, frame.maximized), ((300, 200), (900, 1300), true));
    }
}
