//! Application state types and data structures.
//!
//! This module defines the core data model for the GUI, including:
//! - Root application state
//! - Export statistics
//! - Connection tracking

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use futures::lock::Mutex;
use reliquary_archiver::export::fribbels::OptimizerExporter;
use tokio::sync::watch;
use tracing::level_filters::LevelFilter;

use crate::accounts::MultiAccountManager;
use crate::rgui::components::update::UpdateState;
use crate::rgui::messages::WebSocketStatus;
use crate::rgui::settings::Settings;
use crate::websocket::PortCommand;

/// Information about a detected account.
#[derive(Debug, Clone)]
pub struct AccountInfo {
    pub uid: u32,
}

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
    // Account management
    pub accounts: HashMap<u32, AccountInfo>,
    pub selected_account: Option<u32>,

    // Export data (for currently selected account)
    pub json_export: Option<FileContainer>,
    pub export_out_of_date: bool,
    pub connection_stats: StatsStore,
    pub export_stats: ExportStats,

    pub log_level: LevelFilter,
    pub settings: Settings,
    pub update_state: Option<UpdateState>,
}

impl Default for Store {
    fn default() -> Self {
        Self {
            accounts: HashMap::new(),
            selected_account: None,
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
            relics: exporter.relics().len(),
            characters: exporter.characters().len(),
            light_cones: exporter.light_cones().len(),
            materials: exporter.materials().len(),
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
pub struct RootState {
    /// Multi-account manager for handling multiple exporters
    pub manager: Arc<Mutex<MultiAccountManager>>,
    /// Channel to broadcast GUI's selected account to WebSocket (UI state, not lifecycle events)
    pub selected_account_tx: watch::Sender<Option<u32>>,
    /// Central data store
    pub store: Store,
    /// Whether the settings modal is open
    pub settings_open: bool,
    /// Whether the opacity slider is being dragged (hides modal backdrop)
    pub opacity_slider_dragging: bool,
    /// Channel to send port changes to the WebSocket server
    pub ws_port_sender: Option<watch::Sender<PortCommand>>,
}

impl RootState {
    pub fn new(manager: Arc<Mutex<MultiAccountManager>>) -> Self {
        let (selected_account_tx, _) = watch::channel(None);
        Self {
            manager,
            selected_account_tx,
            store: Store::default(),
            settings_open: false,
            opacity_slider_dragging: false,
            ws_port_sender: None,
        }
    }

    pub fn with_port_sender(self, port_sender: watch::Sender<PortCommand>) -> Self {
        Self {
            ws_port_sender: Some(port_sender),
            ..self
        }
    }

    pub fn with_settings(self, settings: Settings) -> Self {
        Self {
            store: Store { settings, ..self.store },
            ..self
        }
    }
}
