//! Message handlers for processing GUI events.
//!
//! This module contains the handler functions that are called by the main
//! update function to process different message types. Each handler is
//! responsible for updating state and returning optional async tasks.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use chrono::Local;
use futures::channel::oneshot;
use futures::sink::SinkExt;
use raxis::runtime::task::{self, Task};
use reliquary_archiver::export::fribbels::OptimizerEvent;
use tracing::info;

use crate::rgui::messages::{
    AccountMessage, ExportMessage, LogMessage, RootMessage, ScreenAction, WebSocketMessage, WebSocketStatus, WindowMessage,
};
use crate::rgui::state::{AccountInfo, ActiveScreen, ExportStats, FileContainer, FileExtensions, RootState, Screen, WaitingScreen};
use crate::websocket::PortCommand;
use crate::{LOG_BUFFER, VEC_LAYER_HANDLE, worker};

// ============================================================================
// Settings Path Helper
// ============================================================================

/// Returns the path to the settings file within the app data directory.
pub fn get_settings_path(appdata: PathBuf) -> PathBuf {
    appdata.join("reliquary-archiver").join("settings.json")
}

/// Counter for generating unique temp file names within this process.
static SAVE_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Persists the current settings to disk asynchronously.
///
/// Settings are saved atomically to the user's local app data folder as JSON.
/// We write to a temporary file first, then rename it to avoid corruption
/// if the application crashes or is killed during the write.
///
/// Each save uses a unique temp filename (process ID + counter) to prevent
/// race conditions if multiple saves happen concurrently.
pub fn save_settings(state: &RootState) -> Option<Task<RootMessage>> {
    let settings = state.store.settings.clone();
    // Generate unique temp filename to avoid races between concurrent saves
    let unique_id = SAVE_COUNTER.fetch_add(1, Ordering::Relaxed);
    let pid = std::process::id();

    let Some(appdata) = raxis::get_local_app_data() else {
        tracing::error!("Could not determine local app data path, settings not saved");
        return None;
    };

    let path = get_settings_path(appdata);

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

// ============================================================================
// Export Message Handler
// ============================================================================

/// Handles export-related messages.
///
/// Manages export statistics updates, new export creation, and refresh requests.
pub fn handle_export_message(state: &mut RootState, message: ExportMessage) -> Option<Task<RootMessage>> {
    match message {
        ExportMessage::Stats(stats) => {
            state.store.export_stats = stats;
            None
        }

        ExportMessage::New(export) => {
            state.store.json_export = Some(FileContainer {
                name: Local::now().format("archive_output-%Y-%m-%dT%H-%M-%S.json").to_string(),
                content: serde_json::to_string_pretty(&export).unwrap(),
                ext: FileExtensions::of("JSON files", &["json"]),
            });
            state.store.export_out_of_date = false;
            None
        }

        ExportMessage::Refresh => Some(refresh_export_task(state, state.store.selected_account)),
    }
}

/// Asks the worker for a fresh export of the account (`None` = any account) and publishes it once it's ready.
fn refresh_export_task(state: &RootState, uid: Option<u32>) -> Task<RootMessage> {
    let Some(mut sender) = state.worker_sender.clone() else {
        return Task::none();
    };

    Task::future(async move {
        let (tx, rx) = oneshot::channel();
        sender.send(worker::WorkerCommand::MakeExport { uid, sender: tx }).await.ok();
        rx.await.ok().flatten()
    })
    .and_then(|export| Task::done(RootMessage::new_export(export)))
}

/// Reads the current export statistics of the account and publishes them.
fn export_stats_task(state: &RootState, uid: u32) -> Task<RootMessage> {
    let manager = state.manager.clone();

    Task::future(async move {
        let exporter = manager.lock().await.get_account_exporter(uid);
        let stats = match exporter {
            Some(exporter) => {
                let exporter = exporter.lock().await;
                ExportStats::new(&exporter)
            }
            None => ExportStats::default(),
        };
        RootMessage::export_stats(stats)
    })
}

// ============================================================================
// Account Message Handler
// ============================================================================

/// Handles account-related messages.
///
/// Manages account discovery, selection, and stats updates.
pub fn handle_account_message(state: &mut RootState, message: AccountMessage) -> Option<Task<RootMessage>> {
    match message {
        AccountMessage::Discovered { uid } => {
            // Add or update account info
            state.store.accounts.insert(uid, AccountInfo { uid });

            // Always auto-switch to newly discovered account
            Some(select_account(state, uid))
        }

        AccountMessage::Select(uid) => {
            // Only select if account exists
            state.store.accounts.contains_key(&uid).then(|| select_account(state, uid))
        }
    }
}

/// Makes the account the one shown in the GUI and streamed over the WebSocket.
fn select_account(state: &mut RootState, uid: u32) -> Task<RootMessage> {
    state.store.selected_account = Some(uid);
    state.selected_account_tx.send(Some(uid)).ok();
    // Mark export as out of date since it belongs to whatever was selected before
    state.store.export_out_of_date = true;

    // Refresh stats and trigger export for the newly selected account
    Task::batch(vec![export_stats_task(state, uid), refresh_export_task(state, Some(uid))])
}

// ============================================================================
// WebSocket Message Handler
// ============================================================================

/// Handles WebSocket server messages.
///
/// Manages server status updates, port changes, and client connections.
pub fn handle_websocket_message(state: &mut RootState, message: WebSocketMessage) -> Option<Task<RootMessage>> {
    match message {
        WebSocketMessage::Status(status) => {
            state.store.connection_stats.ws_status = status;
            None
        }

        WebSocketMessage::SendPort(port) => {
            if let Some(ref sender) = state.ws_port_sender {
                let _ = sender.send(PortCommand::Open(port));
            }

            // Modify settings but don't save yet to minimize odds of saving on a bad port
            state.store.settings.ws_port = port;
            None
        }

        WebSocketMessage::Close => {
            if let Some(ref sender) = state.ws_port_sender {
                let _ = sender.send(PortCommand::Close);
            }
            None
        }

        WebSocketMessage::PortChanged(port) => {
            // Save settings when the server actually starts rather than when we request a change to avoid saving a bad port
            state.store.connection_stats.ws_status = WebSocketStatus::Running { port, client_count: 0 };
            Some(Task::done(RootMessage::Settings(crate::rgui::messages::SettingsMessage::Save)))
        }

        WebSocketMessage::ClientCountChanged(client_count) => {
            if let WebSocketStatus::Running { port, .. } = state.store.connection_stats.ws_status {
                state.store.connection_stats.ws_status = WebSocketStatus::Running { port, client_count };
            }
            None
        }

        WebSocketMessage::InvalidPort(err) => {
            // If server is already running, don't update status as it will continue running on the previous port
            // If the server is not yet running then update the status with the relevant error message
            if matches!(state.store.connection_stats.ws_status, WebSocketStatus::Pending) {
                state.store.connection_stats.ws_status = WebSocketStatus::Failed { error: err.clone() };
            }
            tracing::info!("Unable to start websocket server on desired port. e={}", err);
            None
        }
    }
}

// ============================================================================
// Log Message Handler
// ============================================================================

/// Handles log viewer messages.
///
/// Manages log level filtering and log export functionality.
pub fn handle_log_message(state: &mut RootState, message: LogMessage) -> Option<Task<RootMessage>> {
    match message {
        LogMessage::LevelChanged(level) => {
            state.store.log_level = level;

            if let Some(handle) = VEC_LAYER_HANDLE.lock().unwrap().as_ref() {
                handle(level);
            }

            None
        }

        LogMessage::Export => Some(
            Task::future(async move {
                if let Some(mut file) = rfd::AsyncFileDialog::new().set_file_name("log.txt").save_file().await {
                    let lines = LOG_BUFFER.lock().unwrap().join("\n");
                    file.write(lines.as_bytes()).await;
                    info!("Exported log to {}", file.path().display());
                }
            })
            .discard(),
        ),
    }
}

// ============================================================================
// Window Message Handler
// ============================================================================

/// Handles window management messages.
///
/// Manages window visibility, settings modal, and tray context menu actions.
pub fn handle_window_message(state: &mut RootState, message: WindowMessage) -> Option<Task<RootMessage>> {
    match message {
        WindowMessage::Hide => Some(task::hide_window()),
        WindowMessage::Show => Some(task::show_window()),
        WindowMessage::ToggleMenu => {
            state.settings_open = !state.settings_open;
            None
        }
        WindowMessage::ContextMenuShow => Some(task::show_window()),
        WindowMessage::ContextMenuMinimize => Some(task::hide_window()),
        WindowMessage::ContextMenuQuit => Some(task::exit_application()),
        WindowMessage::ContextMenuCancelled => None,
    }
}

// ============================================================================
// Connection Check Handler
// ============================================================================

/// Checks for stale connections and updates connection status.
///
/// Marks connections as inactive if no packets/commands received for 60 seconds.
pub fn handle_connection_check(state: &mut RootState, now: Instant) -> Option<Task<RootMessage>> {
    if let Some(last_packet_time) = state.store.connection_stats.last_packet_time {
        if now.duration_since(last_packet_time) > Duration::from_secs(60) {
            state.store.connection_stats.connected = false;
            state.store.connection_stats.connection_active = false;
            state.store.connection_stats.packets_received = 0;
            state.store.connection_stats.commands_received = 0;
        }
    }

    if let Some(last_command_time) = state.store.connection_stats.last_command_time {
        if now.duration_since(last_command_time) > Duration::from_secs(60) {
            state.store.connection_stats.connection_active = false;
        }
    }

    None
}

// ============================================================================
// Worker Event Handler
// ============================================================================

/// Handles events from the background worker thread.
///
/// Processes worker readiness, sniffer metrics, and export events.
pub fn handle_worker_event(state: &mut RootState, event: worker::WorkerEvent) -> Task<RootMessage> {
    match event {
        worker::WorkerEvent::Ready(sender) => {
            state.worker_sender = Some(sender);
        }
        worker::WorkerEvent::AccountReconnected { uid } => {
            // Refresh stats for this account
            return export_stats_task(state, uid);
        }
        worker::WorkerEvent::Metric(metric) => {
            handle_sniffer_metric(state, metric);
        }
        worker::WorkerEvent::AccountDiscovered { uid } => {
            return Task::done(RootMessage::Account(AccountMessage::Discovered { uid }));
        }
        worker::WorkerEvent::ExportEvent { uid, event } => {
            // Only process if this is the selected account
            let is_selected = state.store.selected_account == Some(uid) || state.store.selected_account.is_none();

            if !is_selected {
                // Event for different account, ignore
                return Task::none();
            }

            // Mark export as out of date
            state.store.export_out_of_date = true;

            let export_task = match event {
                OptimizerEvent::InitialScan(scan) => Task::done(RootMessage::new_export(scan)),
                _ => Task::none(),
            };

            // Update stats from the manager
            return Task::batch([export_task, export_stats_task(state, uid)]);
        }
    }

    Task::none()
}

/// Updates connection statistics based on sniffer metrics.
///
/// Tracks connection state, packet counts, and error conditions.
pub fn handle_sniffer_metric(state: &mut RootState, metric: worker::SnifferMetric) {
    let stats = &mut state.store.connection_stats;

    match metric {
        worker::SnifferMetric::ConnectionEstablished => {
            stats.connected = true;
        }
        worker::SnifferMetric::ConnectionDisconnected => {
            stats.connected = false;
            stats.connection_active = false;
            stats.packets_received = 0;
            stats.commands_received = 0;
        }
        worker::SnifferMetric::NetworkPacketReceived => {
            stats.connected = true;
            stats.last_packet_time = Some(std::time::Instant::now());
            stats.packets_received += 1;
        }
        worker::SnifferMetric::GameCommandsReceived(commands) => {
            if commands > 0 {
                stats.connected = true;
                stats.connection_active = true;
                stats.commands_received += commands;
                stats.last_command_time = Some(std::time::Instant::now());
            }
        }
        worker::SnifferMetric::DecryptionKeyMissing => {
            stats.decryption_key_missing += 1;
        }
        worker::SnifferMetric::NetworkError => {
            stats.network_errors += 1;
        }
    }
}

// ============================================================================
// Screen Transitions
// ============================================================================

/// Manages automatic screen transitions based on connection state.
///
/// Switches between Waiting and Active screens when connection status changes.
pub fn handle_screen_transitions(state: &mut RootState) {
    let is_connected = state.store.connection_stats.connection_active;
    let is_waiting = matches!(&state.screen, Screen::Waiting(_));
    if is_connected && is_waiting {
        state.screen = Screen::Active(ActiveScreen::new());
    } else if !is_connected && !is_waiting {
        state.screen = Screen::Waiting(WaitingScreen::new());
    }
}

// ============================================================================
// Screen Action Implementation
// ============================================================================

impl<Message: Send + 'static> ScreenAction<Message> {
    pub fn run(self, state: &mut RootState, wrapper: impl Send + Fn(Message) -> RootMessage + 'static) -> Task<RootMessage> {
        match self {
            Self::None => Task::none(),
            Self::Run(task) => task.map(wrapper),
            #[cfg(feature = "pcap")]
            Self::ProcessCapture(path) => Task::future(crate::session::export_from_file(crate::capture::SourceSpec::PcapFile(path)))
                .and_then(|e| Task::done(RootMessage::new_export(e))),
        }
    }
}
