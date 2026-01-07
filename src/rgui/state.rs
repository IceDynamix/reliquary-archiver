//! Application state types and data structures.
//!
//! This module defines the core data model for the GUI, including:
//! - Root application state
//! - User settings (persisted to disk)
//! - Export statistics
//! - Connection tracking
//! - Screen states

use std::sync::Arc;
use std::time::Instant;

use futures::lock::Mutex;
use tokio::sync::watch::Sender;
use tracing::level_filters::LevelFilter;
use reliquary_archiver::export::fribbels::OptimizerExporter;

use crate::worker;
use crate::rgui::messages::WebSocketStatus;
use crate::rgui::components::update::UpdateState;

/// File extension filter configuration for save dialogs.
#[derive(Debug, Clone)]
pub struct FileExtensions {
    pub description: String,
    pub extensions: Vec<String>,
}

impl FileExtensions {
    pub fn of(description: &str, extensions: &[&str]) -> Self {
        Self {
            description: description.to_string(),
            extensions: extensions.iter().map(|e| e.to_string()).collect(),
        }
    }
}

/// Container for file data to be saved or downloaded.
///
/// Used primarily for JSON exports that can be downloaded by the user.
#[derive(Debug, Clone)]
pub struct FileContainer {
    /// Suggested filename for the save dialog
    pub name: String,
    /// The file's content as a string
    pub content: String,
    /// File extension filter for the save dialog
    pub ext: FileExtensions,
}

/// Central data store containing all persistent and runtime state.
///
/// This is the "model" part of the MVU architecture, containing all
/// application data that the view reads from.
pub struct Store {
    pub json_export: Option<FileContainer>,
    pub export_out_of_date: bool,
    pub connection_stats: StatsStore,
    pub export_stats: ExportStats,

    pub log_level: LevelFilter,
    pub settings: Settings,
    pub update_state: Option<UpdateState>,
}

/// Background image scaling mode.
///
/// Controls how a custom background image is displayed within the window.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize, Default)]
pub enum ImageFit {
    /// Stretch to fill the container (default)
    #[default]
    Fill,
    /// Scale to fit inside the container while maintaining aspect ratio
    Contain,
    /// Scale to cover the container while maintaining aspect ratio (may crop)
    Cover,
    /// Like contain but never scale up beyond intrinsic size
    ScaleDown,
    /// Display at intrinsic size with no scaling
    None,
}

/// User-configurable application settings.
///
/// These settings are persisted to disk as JSON in the user's app data folder.
/// Settings are automatically saved when modified through the settings modal.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
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
    pub ws_port: u16
}

impl Default for Store {
    fn default() -> Self {
        Self {
            json_export: None,
            export_out_of_date: false,
            connection_stats: StatsStore::default(),
            export_stats: ExportStats::default(),
            log_level: LevelFilter::INFO, // TODO: This is not right
            settings: Settings::default(),
            update_state: None,
        }
    }
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            background_image: "".to_string(),
            image_fit: ImageFit::Cover,
            background_opacity: 0.12,
            text_shadow_enabled: false,
            always_update: false,
            minimize_to_tray_on_close: false,
            minimize_to_tray_on_minimize: false,
            run_on_start: false,
            start_minimized: false,
            ws_port: 23313
        }
    }
}

/// Statistics about the current export data.
///
/// Displayed on the active screen to show what has been captured.
#[derive(Default, Debug, Clone, Copy)]
pub struct ExportStats {
    /// Number of relics captured
    pub relics: usize,
    /// Number of characters captured
    pub characters: usize,
    /// Number of light cones captured
    pub light_cones: usize,
    /// Number of materials captured
    pub materials: usize,
}

impl ExportStats {
    pub fn new(exporter: &OptimizerExporter) -> Self {
        Self {
            relics: exporter.relics.len(),
            characters: exporter.characters.len(),
            light_cones: exporter.light_cones.len(),
            materials: exporter.materials.len(),
        }
    }
}

/// Connection and network statistics.
///
/// Tracks the state of the game connection and WebSocket server.
#[derive(Default)]
pub struct StatsStore {
    /// Current WebSocket server status
    pub ws_status: WebSocketStatus,
    /// Whether we have an active network capture connection
    pub connected: bool,
    /// Whether we're actively receiving game data
    pub connection_active: bool,
    /// Total packets received from the network
    pub packets_received: usize,
    /// Total game commands decoded
    pub commands_received: usize,
    /// Count of packets that couldn't be decrypted
    pub decryption_key_missing: usize,
    /// Count of network errors encountered
    pub network_errors: usize,
    /// Timestamp of the last received packet
    pub last_packet_time: Option<Instant>,
    /// Timestamp of the last decoded command
    pub last_command_time: Option<Instant>,
}

/// Root application state containing all UI and runtime data.
///
/// This is the top-level state object passed to the view and update functions.
#[derive(Default)]
pub struct RootState {
    /// Shared exporter instance for generating output files
    pub exporter: Arc<Mutex<OptimizerExporter>>,
    /// Channel to send commands to the background worker
    pub worker_sender: Option<worker::WorkerHandle>,
    /// Central data store
    pub store: Store,
    /// Current screen being displayed
    pub screen: Screen,
    /// Whether the settings modal is open
    pub settings_open: bool,
    /// Whether the opacity slider is being dragged (hides modal backdrop)
    pub opacity_slider_dragging: bool,
    /// Channel to send port changes to the WebSocket server
    pub ws_port_sender: Option<Sender<u16>>,
}

impl RootState {
    pub fn with_port_sender(self: Self, port_sender: Sender<u16>) -> Self {
        Self {
            ws_port_sender: Some(port_sender),
            ..self
        }
    }
}

/// Application screen states.
///
/// The app transitions between screens based on connection status.
#[derive(Debug)]
pub enum Screen {
    /// Waiting for game connection - shown when not connected
    Waiting(WaitingScreen),
    /// Active capture screen - shown when connected to the game
    Active(ActiveScreen),
}

impl Default for Screen {
    fn default() -> Self {
        Self::Waiting(WaitingScreen::new())
    }
}

/// State for the waiting screen (pre-connection).
#[derive(Default, Debug)]
pub struct WaitingScreen {
    // Waiting screen specific state
}

impl WaitingScreen {
    pub fn new() -> Self {
        Self::default()
    }
}

/// State for the active screen (connected and capturing).
#[derive(Default, Debug)]
pub struct ActiveScreen {
    // Active screen specific state
}

impl ActiveScreen {
    pub fn new() -> Self {
        Self::default()
    }
}
