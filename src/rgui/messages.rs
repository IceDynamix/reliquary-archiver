use std::path::PathBuf;
use std::time::Instant;

use raxis::runtime::task::Task;
use tracing::level_filters::LevelFilter;
use reliquary_archiver::export::fribbels::Export;

use crate::worker;
use crate::rgui::state::{ExportStats, ImageFit, Settings};
use crate::rgui::components::update::UpdateMessage;

// ============================================================================
// WebSocket Messages
// ============================================================================

#[derive(Debug, Clone, Default)]
pub enum WebSocketStatus {
    #[default]
    Pending,
    Running {
        port: u16,
        client_count: usize,
    },
    Failed {
        error: String,
    },
}

#[derive(Debug, Clone)]
pub enum WebSocketMessage {
    Status(WebSocketStatus),
    SendPort(u16),
    PortChanged(u16),
    ClientCountChanged(usize),
    InvalidPort(String),
}

// ============================================================================
// Export Messages
// ============================================================================

#[derive(Debug, Clone)]
pub enum ExportMessage {
    Stats(ExportStats),
    New(Export),
    Refresh,
}

// ============================================================================
// Settings Messages
// ============================================================================

#[derive(Debug, Clone)]
pub enum SettingsMessage {
    // Persistence
    Load(PathBuf),
    Activate(Settings),
    Save,
    
    // Graphics settings
    BackgroundImageSelected(Option<PathBuf>),
    RemoveBackgroundImage,
    ImageFitChanged(ImageFit),
    OpacityChanged(f32),
    OpacitySliderDrag(bool),
    TextShadowToggled(bool),
    
    // Update settings
    AlwaysUpdateToggled(bool),
    
    // Window behavior settings
    MinimizeToTrayOnCloseToggled(bool),
    MinimizeToTrayOnMinimizeToggled(bool),
    RunOnStartToggled(bool),
    StartMinimizedToggled(bool),
}

// ============================================================================
// Window Messages
// ============================================================================

#[derive(Debug, Clone)]
pub enum WindowMessage {
    Hide,
    Show,
    ToggleMenu,
    ContextMenuShow,
    ContextMenuMinimize,
    ContextMenuQuit,
    ContextMenuCancelled,
}

// ============================================================================
// Log Messages
// ============================================================================

#[derive(Debug, Clone)]
pub enum LogMessage {
    LevelChanged(LevelFilter),
    Export,
}

// ============================================================================
// Screen Messages
// ============================================================================

#[derive(Debug, Clone)]
pub enum WaitingMessage {
    PcapFileSelected(Option<PathBuf>),
}

#[derive(Debug, Clone)]
pub enum ActiveMessage {
    // Active screen messages
}

pub enum ScreenAction<Message> {
    None,
    Run(Task<Message>),
    RefreshExport,
    #[cfg(feature = "pcap")]
    ProcessCapture(PathBuf),
}

// ============================================================================
// Root Message - Aggregates all message types
// ============================================================================

#[derive(Debug, Clone)]
pub enum RootMessage {
    // Simple triggers
    TriggerRender,
    GoToLink(String),
    
    // Worker/connection events
    WorkerEvent(worker::WorkerEvent),
    CheckConnection(Instant),
    
    // Grouped messages
    Export(ExportMessage),
    WebSocket(WebSocketMessage),
    Settings(SettingsMessage),
    Window(WindowMessage),
    Log(LogMessage),
    Update(UpdateMessage),
    
    // Screen messages
    WaitingScreen(WaitingMessage),
    ActiveScreen(ActiveMessage),
}

// ============================================================================
// Convenience constructors for common message patterns
// ============================================================================

impl RootMessage {
    // WebSocket shortcuts
    pub fn ws_status(status: WebSocketStatus) -> Self {
        Self::WebSocket(WebSocketMessage::Status(status))
    }
    
    pub fn ws_port_changed(port: u16) -> Self {
        Self::WebSocket(WebSocketMessage::PortChanged(port))
    }
    
    pub fn ws_client_count_changed(count: usize) -> Self {
        Self::WebSocket(WebSocketMessage::ClientCountChanged(count))
    }
    
    pub fn ws_invalid_port(err: String) -> Self {
        Self::WebSocket(WebSocketMessage::InvalidPort(err))
    }
    
    // Export shortcuts
    pub fn new_export(export: Export) -> Self {
        Self::Export(ExportMessage::New(export))
    }
    
    pub fn export_stats(stats: ExportStats) -> Self {
        Self::Export(ExportMessage::Stats(stats))
    }
}
