use std::sync::Arc;
use std::time::Instant;

use futures::lock::Mutex;
use tokio::sync::watch::Sender;
use tracing::level_filters::LevelFilter;
use reliquary_archiver::export::fribbels::OptimizerExporter;

use crate::worker;
use crate::rgui::messages::WebSocketStatus;
use crate::rgui::components::update::UpdateState;

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

#[derive(Debug, Clone)]
pub struct FileContainer {
    pub name: String,
    pub content: String,
    pub ext: FileExtensions,
}

// State Management
pub struct Store {
    pub json_export: Option<FileContainer>,
    pub export_out_of_date: bool,
    pub connection_stats: StatsStore,
    pub export_stats: ExportStats,

    pub log_level: LevelFilter,
    pub settings: Settings,
    pub update_state: Option<UpdateState>,
}

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

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Settings {
    pub background_image: String,
    pub image_fit: ImageFit,
    pub background_opacity: f32,
    pub text_shadow_enabled: bool,
    pub always_update: bool,
    pub minimize_to_tray_on_close: bool,
    pub minimize_to_tray_on_minimize: bool,
    pub run_on_start: bool,
    pub start_minimized: bool,
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

#[derive(Default, Debug, Clone, Copy)]
pub struct ExportStats {
    pub relics: usize,
    pub characters: usize,
    pub light_cones: usize,
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

#[derive(Default)]
pub struct StatsStore {
    pub ws_status: WebSocketStatus,
    pub connected: bool,
    pub connection_active: bool,
    pub packets_received: usize,
    pub commands_received: usize,
    pub decryption_key_missing: usize,
    pub network_errors: usize,
    pub last_packet_time: Option<Instant>,
    pub last_command_time: Option<Instant>,
}

#[derive(Default)]
pub struct RootState {
    pub exporter: Arc<Mutex<OptimizerExporter>>,
    pub worker_sender: Option<worker::WorkerHandle>,
    pub store: Store,
    pub screen: Screen,
    pub settings_open: bool,
    pub opacity_slider_dragging: bool,
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

#[derive(Debug)]
pub enum Screen {
    Waiting(WaitingScreen),
    Active(ActiveScreen),
}

impl Default for Screen {
    fn default() -> Self {
        Self::Waiting(WaitingScreen::new())
    }
}

#[derive(Default, Debug)]
pub struct WaitingScreen {
    // Waiting screen specific state
}

impl WaitingScreen {
    pub fn new() -> Self {
        Self::default()
    }
}

#[derive(Default, Debug)]
pub struct ActiveScreen {
    // Active screen specific state
}

impl ActiveScreen {
    pub fn new() -> Self {
        Self::default()
    }
}
