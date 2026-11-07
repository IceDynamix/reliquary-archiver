//! Message types for the GUI event system.
//!
//! Messages represent all possible events that can occur in the application.
//! They are dispatched to the update function to modify state.
//!
//! Messages are organized into groups for related functionality:
//! - [`WebSocketMessage`]: WebSocket server events
//! - [`ExportMessage`]: Export generation events
//! - [`SettingsMessage`]: Settings changes
//! - [`WindowMessage`]: Window management events
//! - [`LogMessage`]: Log viewer events
//! - [`RootMessage`]: Top-level message aggregator

use std::path::PathBuf;
use std::time::Instant;

use reliquary_archiver::export::fribbels::Export;
use tracing::level_filters::LevelFilter;

use crate::rgui::components::update::UpdateMessage;
use crate::rgui::settings::{ImageFit, Settings};
use crate::rgui::state::ExportStats;
use crate::rgui::worker;

// ============================================================================
// WebSocket Messages
// ============================================================================

/// Current status of the WebSocket server.
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

/// Messages related to the WebSocket server for external tool integration.
#[derive(Debug, Clone)]
pub enum WebSocketMessage {
    /// Update the server status
    Status(WebSocketStatus),
    /// Request to change the server port
    SendPort(u16),
    /// Server successfully started on a new port
    PortChanged(u16),
    /// Number of connected clients changed
    ClientCountChanged(usize),
    /// Failed to bind to the requested port
    InvalidPort(String),
    /// Close the WebSocket server
    Close,
}

// ============================================================================
// Export Messages
// ============================================================================

/// Messages related to export file generation.
#[derive(Debug, Clone)]
pub enum ExportMessage {
    /// Update export statistics display
    Stats(ExportStats),
    /// A new export is ready for download
    New(Export),
    /// Request to regenerate the export from current data
    Refresh,
    /// Export the account found in a recorded capture file
    LoadCaptureFile(PathBuf),
}

// ============================================================================
// Settings Messages
// ============================================================================

/// Messages for settings changes.
#[derive(Debug, Clone)]
pub enum SettingsMessage {
    // Graphics settings
    /// User selected a new background image
    BackgroundImageSelected(Option<PathBuf>),
    /// Clear the background image
    RemoveBackgroundImage,
    /// Change how the background image is scaled
    ImageFitChanged(ImageFit),
    /// Background opacity slider value changed
    OpacityChanged(f32),
    /// Opacity slider drag state changed (for hiding modal backdrop)
    OpacitySliderDrag(bool),
    /// Toggle text shadow effect
    TextShadowToggled(bool),

    // Update settings
    /// Toggle automatic updates
    AlwaysUpdateToggled(bool),

    // Window behavior settings
    /// Toggle minimize to tray on close
    MinimizeToTrayOnCloseToggled(bool),
    /// Toggle minimize to tray on minimize
    MinimizeToTrayOnMinimizeToggled(bool),
    /// Toggle run on Windows startup
    RunOnStartToggled(bool),
    /// Toggle starting minimized
    StartMinimizedToggled(bool),
}

// ============================================================================
// Window Messages
// ============================================================================

/// Messages for window management and tray context menu actions.
#[derive(Debug, Clone)]
pub enum WindowMessage {
    /// Hide the window (minimize to tray)
    Hide,
    /// Show/restore the window
    Show,
    /// Toggle the settings modal
    ToggleMenu,
    /// Tray context menu: show window
    ContextMenuShow,
    /// Tray context menu: minimize to tray
    ContextMenuMinimize,
    /// Tray context menu: exit application
    ContextMenuQuit,
    /// Tray context menu was dismissed
    ContextMenuCancelled,
}

// ============================================================================
// Log Messages
// ============================================================================

/// Messages for the log viewer component.
#[derive(Debug, Clone)]
pub enum LogMessage {
    /// Change the log level filter
    LevelChanged(LevelFilter),
    /// Export logs to a file
    Export,
}

// ============================================================================
// Account Messages
// ============================================================================

/// Messages related to account management.
#[derive(Debug, Clone)]
pub enum AccountMessage {
    /// An account logged in, for the first time or again
    LoggedIn { uid: u32 },
    /// User selected a different account to view
    Select(u32),
}

// ============================================================================
// Root Message - Aggregates all message types
// ============================================================================

/// Top-level message type aggregating all message categories.
///
/// This is the main message type used throughout the application.
/// All events flow through this enum to the update function.
#[derive(Debug, Clone)]
pub enum RootMessage {
    // Simple triggers
    /// Force a UI re-render
    TriggerRender,
    /// Open a URL in the default browser
    GoToLink(String),

    // Worker/connection events
    /// Event from the background worker
    WorkerEvent(worker::WorkerEvent),
    /// Periodic connection status check
    CheckConnection(Instant),

    // Grouped messages
    /// Account management messages
    Account(AccountMessage),
    /// Export-related messages
    Export(ExportMessage),
    /// WebSocket server messages
    WebSocket(WebSocketMessage),
    /// Settings messages
    Settings(SettingsMessage),
    /// Window management messages
    Window(WindowMessage),
    /// Log viewer messages
    Log(LogMessage),
    /// Update checker messages
    Update(UpdateMessage),
}

// ============================================================================
// Convenience constructors for common message patterns
// ============================================================================

impl RootMessage {
    /// Create a WebSocket status update message.
    pub fn ws_status(status: WebSocketStatus) -> Self {
        Self::WebSocket(WebSocketMessage::Status(status))
    }

    /// Create a WebSocket port changed message.
    pub fn ws_port_changed(port: u16) -> Self {
        Self::WebSocket(WebSocketMessage::PortChanged(port))
    }

    /// Create a WebSocket client count changed message.
    pub fn ws_client_count_changed(count: usize) -> Self {
        Self::WebSocket(WebSocketMessage::ClientCountChanged(count))
    }

    /// Create a WebSocket invalid port error message.
    pub fn ws_invalid_port(err: String) -> Self {
        Self::WebSocket(WebSocketMessage::InvalidPort(err))
    }

    /// Create a new export message.
    pub fn new_export(export: Export) -> Self {
        Self::Export(ExportMessage::New(export))
    }

    /// Create an export stats update message.
    pub fn export_stats(stats: ExportStats) -> Self {
        Self::Export(ExportMessage::Stats(stats))
    }
}
