//! Message handlers for processing GUI events.
//!
//! This module contains the handler functions that are called by the main
//! update function to process different message types. Each handler is
//! responsible for updating state and returning optional async tasks.

use std::time::{Duration, Instant};

use chrono::Local;
use raxis::runtime::task::{self, Task};
use reliquary_archiver::export::Exporter;
use reliquary_archiver::export::fribbels::OptimizerEvent;
use tracing::info;

use crate::capture::Replay;
use crate::logging;
use crate::rgui::components::update::{self, UpdateMessage};
use crate::rgui::messages::{AccountMessage, ExportMessage, LogMessage, RootMessage, WebSocketMessage, WebSocketStatus, WindowMessage};
use crate::rgui::state::{AccountInfo, ExportStats, FileContainer, FileExtensions, RootState};
use crate::rgui::worker::WorkerEvent;
use crate::session::{self, SessionEvent};
use crate::websocket::PortCommand;

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

        ExportMessage::LoadCaptureFile(path) => {
            let Some(replay) = Replay::for_path(path.clone()) else {
                tracing::warn!("Can't read capture files like {}", path.display());
                return None;
            };

            tracing::info!("Processing capture file: {}", path.display());
            Some(Task::future(session::export_from_file(replay)).and_then(|export| Task::done(RootMessage::new_export(export))))
        }
    }
}

/// Exports the account (`None` = any account) and publishes the export, once everything needed for one has been captured.
fn refresh_export_task(state: &RootState, uid: Option<u32>) -> Task<RootMessage> {
    let manager = state.manager.clone();

    Task::future(async move {
        let exporter = {
            let manager = manager.lock().await;
            match uid {
                Some(uid) => manager.get_account_exporter(uid),
                None => manager.get_first_account_exporter(),
            }
        }?;

        let exporter = exporter.lock().await;
        if exporter.is_initialized() { exporter.export() } else { None }
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
        AccountMessage::LoggedIn { uid } => {
            // Add or update account info
            state.store.accounts.insert(uid, AccountInfo { uid });

            // Always auto-switch to the account that logged in last
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
            state.ws_port_sender.send(PortCommand::Open(port)).ok();

            // Only make it a setting once the server runs on it, so a bad port is never saved
            state.pending_ws_port = Some(port);
            None
        }

        WebSocketMessage::Close => {
            state.ws_port_sender.send(PortCommand::Close).ok();
            None
        }

        WebSocketMessage::PortChanged(port) => {
            state.store.connection_stats.ws_status = WebSocketStatus::Running { port, client_count: 0 };

            // Once the server runs on the requested port, it becomes the setting. Keep the requested port
            // rather than `port`, so that 0 (pick any free port) stays 0. A port from an earlier request
            // leaves the latest one pending
            if let Some(requested) = state.pending_ws_port
                && (requested == 0 || requested == port)
            {
                state.store.settings.ws_port = requested;
                state.pending_ws_port = None;
            }
            None
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
            state.pending_ws_port = None;
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

            logging::set_console_level(level);

            None
        }

        LogMessage::Export => Some(
            Task::future(async move {
                if let Some(mut file) = rfd::AsyncFileDialog::new().set_file_name("log.txt").save_file().await {
                    let lines = logging::lines().join("\n");
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
            state.store.connection_stats.capture_stalled = false;
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
/// Processes logins, capture statistics, and export events.
pub fn handle_worker_event(state: &mut RootState, event: WorkerEvent) -> Task<RootMessage> {
    match event {
        WorkerEvent::PacketReceived => {
            let stats = &mut state.store.connection_stats;
            stats.connected = true;
            stats.last_packet_time = Some(Instant::now());
            stats.packets_received += 1;
        }
        WorkerEvent::Session(event) => handle_session_event(state, event),
        WorkerEvent::AccountLoggedIn { uid } => {
            return Task::done(RootMessage::Account(AccountMessage::LoggedIn { uid }));
        }
        WorkerEvent::ExportEvent { uid, event } => {
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

/// Updates connection statistics based on what the capture session reported.
///
/// Tracks connection state, packet counts, and error conditions.
fn handle_session_event(state: &mut RootState, event: SessionEvent) {
    let stats = &mut state.store.connection_stats;

    match event {
        SessionEvent::ConnectionEstablished => {
            stats.connected = true;
        }
        SessionEvent::Disconnected => {
            stats.connected = false;
            stats.connection_active = false;
            stats.packets_received = 0;
            stats.commands_received = 0;
        }
        SessionEvent::PacketsDecoded(commands) => {
            if commands > 0 {
                stats.connected = true;
                stats.connection_active = true;
                stats.commands_received += commands;
                stats.last_command_time = Some(Instant::now());
            }
        }
        SessionEvent::DecryptionKeyMissing => {
            stats.decryption_key_missing += 1;
        }
        SessionEvent::DecodeError => {
            stats.network_errors += 1;
        }
        SessionEvent::Stalled => {
            stats.capture_stalled = true;
        }
        SessionEvent::Recovered => {
            stats.capture_stalled = false;
        }
        // Logins reach the GUI through the account manager, as WorkerEvent::AccountLoggedIn
        SessionEvent::AccountSeen { .. } | SessionEvent::LoginFinished => {}
    }
}

// ============================================================================
// Update Message Handler
// ============================================================================

/// Handles update checker messages.
///
/// Closes the WebSocket server when the user confirms an update, and exits once the update is ready to restart.
pub fn handle_update_message(state: &mut RootState, message: UpdateMessage) -> Option<Task<RootMessage>> {
    let close_websocket = if matches!(message, UpdateMessage::Confirm) {
        Task::done(RootMessage::WebSocket(WebSocketMessage::Close))
    } else {
        Task::none()
    };

    let update_task = match update::handle_message(message, &mut state.store.update_state, state.store.settings.always_update) {
        update::HandleResult::None => Task::none(),
        update::HandleResult::Task(update_task) => update_task.map(RootMessage::Update),
        update::HandleResult::ExitForRestart => return Some(task::exit_application()),
    };

    Some(close_websocket.chain(update_task))
}
