//! # Reliquary Archiver GUI Module
//!
//! This module provides the graphical user interface.
//!
//! ## Architecture
//!
//! The GUI follows the Model-View-Update (MVU) pattern:
//! - **State** ([`RootState`]): The application's data model
//! - **Messages** ([`RootMessage`]): Events that trigger state changes
//! - **Update** ([`update`]): Processes messages and updates state
//! - **View** ([`view`]): Renders the UI based on current state
//!
//! ## Module Structure
//!
//! - [`state`]: Application state types and data structures
//! - [`messages`]: Message types for event handling
//! - [`handlers`]: Message handler implementations
//! - [`view`]: Main view rendering function
//! - [`theme`]: UI theming constants and helpers
//! - [`components`]: Reusable UI components (settings modal, log view, etc.)
//! - [`screens`]: Screen-specific views (waiting, active)
//! - [`kit`]: Low-level UI building blocks (icons, modals, toggles, tooltips)
//! - [`run_on_start`]: Windows startup registry integration

use std::time::Instant;

use async_stream::stream;
use raxis::runtime::task::{self, Task};
use raxis::runtime::window::builder::InitialDisplay;
use raxis::runtime::Backdrop;
use raxis::{ContextMenuItem, SystemCommand, SystemCommandResponse, TrayEvent, TrayIconConfig};
use tokio::sync::watch;
use tokio_stream::wrappers::WatchStream;

use crate::rgui::components::update::UpdateMessage;
use crate::scopefns::Also;
use crate::websocket::{start_websocket_server, PortCommand, PortSource};
use crate::worker::archiver_worker;
use crate::LOG_NOTIFY;

// Module declarations
pub mod components;
pub mod handlers;
pub mod kit;
pub mod messages;
pub mod run_on_start;
pub mod screens;
pub mod state;
pub mod theme;
pub mod view;

// Re-exports for public API
pub use handlers::{
    get_settings_path, handle_connection_check, handle_export_message, handle_log_message, handle_screen_transitions,
    handle_sniffer_metric, handle_websocket_message, handle_window_message, handle_worker_event, save_settings,
};
pub use messages::*;
pub use state::*;
pub use theme::*;
pub use view::view;

/// Main update function that processes messages and modifies application state.
///
/// This is the core of the Elm architecture - all state changes flow through here.
/// Messages are dispatched to appropriate handlers based on their type.
///
/// # Returns
/// An optional [`Task`] to be executed asynchronously (e.g., I/O operations, timers).
pub fn update(state: &mut RootState, message: RootMessage) -> Option<Task<RootMessage>> {
    macro_rules! handle_screen {
        ($screen:ident, $screen_message:ident, $message:ident) => {
            if let Screen::$screen(screen) = &mut state.screen {
                Some(screen.update($message).run(state, RootMessage::$screen_message))
            } else {
                None
            }
        };
    }

    match message {
        // Simple triggers
        RootMessage::TriggerRender => None,

        RootMessage::GoToLink(link) => {
            if let Err(e) = open::that(link) {
                tracing::error!("Failed to open link: {}", e);
            }
            None
        }

        // Worker/connection events
        RootMessage::WorkerEvent(event) => Some(handle_worker_event(state, event)),
        RootMessage::CheckConnection(now) => handle_connection_check(state, now),

        // Grouped message handlers - delegate to appropriate modules
        RootMessage::Account(msg) => handlers::handle_account_message(state, msg),
        RootMessage::Export(msg) => handle_export_message(state, msg),
        RootMessage::WebSocket(msg) => handle_websocket_message(state, msg),
        RootMessage::Settings(msg) => components::settings_modal::handle_settings_message(state, msg),
        RootMessage::Window(msg) => handle_window_message(state, msg),
        RootMessage::Log(msg) => handle_log_message(state, msg),

        // Screen messages
        RootMessage::WaitingScreen(message) => handle_screen!(Waiting, WaitingScreen, message),
        RootMessage::ActiveScreen(message) => handle_screen!(Active, ActiveScreen, message),

        // Update messages
        RootMessage::Update(msg) => {
            use crate::rgui::components::update;
            let task = if matches!(msg, UpdateMessage::Confirm) {
                Task::done(RootMessage::WebSocket(WebSocketMessage::Close))
            } else {
                Task::none()
            };
            Some(
                task.chain(
                    match update::handle_message(msg, &mut state.store.update_state, state.store.settings.always_update) {
                        update::HandleResult::None => None,
                        update::HandleResult::Task(t) => Some(t.map(RootMessage::Update)),
                        update::HandleResult::ExitForRestart => return Some(task::exit_application()),
                    }
                    .unwrap_or(Task::none()),
                ),
            )
        }
    }
    .also(|_| {
        handle_screen_transitions(state);
    })
}

/// Initializes and runs the application GUI.
///
/// This function sets up the raxis application with:
/// - Window configuration (size, title, backdrop effects)
/// - System tray integration with context menu
/// - WebSocket server for external tool integration
/// - Background worker for packet capture processing
/// - Update checking on startup
///
/// # Errors
/// Returns an error if the GUI framework fails to initialize.
pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    use std::sync::Arc;

    use futures::lock::Mutex;
    use reliquary_archiver::export::database::get_database;

    use crate::rgui::components::update;
    use crate::worker::MultiAccountManager;

    let (port_tx, port_rx) = watch::channel::<PortCommand>(PortCommand::Open(0));

    let database = get_database();
    let manager = Arc::new(Mutex::new(MultiAccountManager::new(database.keys.clone())));

    let state = RootState::new(manager.clone()).with_port_sender(port_tx);

    let app = raxis::Application::new(state, view, update, move |state| {
        let selected_account_rx = state.selected_account_tx.subscribe();

        Some(Task::batch(vec![
            task::get_local_app_data().and_then(|path| Task::done(RootMessage::Settings(SettingsMessage::Load(get_settings_path(path))))),
            // technically calling PerformCheck here is a data race (should be .chain() to SettingsMessage::Activate)
            // however its a case of loading+parsing ~300 bytes of json vs a round trip web request
            Task::done(RootMessage::Update(update::UpdateMessage::PerformCheck)),
            Task::run(archiver_worker(manager.clone()), RootMessage::WorkerEvent),
            Task::future(start_websocket_server(
                PortSource::Dynamic(WatchStream::from_changes(port_rx.clone())),
                manager.clone(),
                selected_account_rx,
            ))
            .then(|e| match e {
                Err(e) => Task::done(RootMessage::ws_status(WebSocketStatus::Failed { error: e })),
                Ok((port_stream, client_count_stream)) => {
                    Task::done(RootMessage::ws_status(WebSocketStatus::Pending)).chain(Task::batch(vec![
                        Task::stream(client_count_stream).map(RootMessage::ws_client_count_changed),
                        Task::stream(port_stream).map(|port| match port {
                            Ok(port) => RootMessage::ws_port_changed(port),
                            Err(e) => RootMessage::ws_invalid_port(e),
                        }),
                    ]))
                }
            }),
            Task::stream(stream! {
                loop {
                    LOG_NOTIFY.notified().await;
                    yield RootMessage::TriggerRender;
                }
            }),
        ]))
    })
    .with_title("Reliquary Archiver")
    .with_icons(Some(1))
    .with_tray_icon(TrayIconConfig {
        icon_resource: Some(1),
        tooltip: Some("Reliquary Archiver".to_string()),
    })
    .with_tray_event_handler(|state, event| match event {
        TrayEvent::LeftClick | TrayEvent::LeftDoubleClick => Some(task::get_window_mode().then({
            // TODO: Does this make sense or should it also consider onClose preference (re)
            let minimize_to_tray = state.store.settings.minimize_to_tray_on_minimize;
            move |mode| {
                use raxis::runtime::task::WindowMode;
                match mode {
                    WindowMode::Hidden => task::show_window(),
                    WindowMode::Windowed => {
                        if minimize_to_tray {
                            task::hide_window()
                        } else {
                            task::minimize_window()
                        }
                    }
                    WindowMode::Minimized => task::restore_window(),
                }
            }
        })),
        TrayEvent::RightClick => Some(task::get_window_mode().then(|mode| {
            use raxis::runtime::task::WindowMode;
            task::show_context_menu(
                vec![
                    if mode == WindowMode::Hidden {
                        ContextMenuItem::new(RootMessage::Window(WindowMessage::ContextMenuShow), "Show Window")
                    } else {
                        ContextMenuItem::new(RootMessage::Window(WindowMessage::ContextMenuMinimize), "Minimize to Tray")
                    },
                    ContextMenuItem::separator(),
                    ContextMenuItem::new(RootMessage::Window(WindowMessage::ContextMenuQuit), "Quit"),
                ],
                RootMessage::Window(WindowMessage::ContextMenuCancelled),
            )
        })),
    })
    .with_syscommand_handler(|state, command| match command {
        SystemCommand::Close => {
            if state.store.settings.minimize_to_tray_on_close {
                return SystemCommandResponse::PreventWith(RootMessage::Window(WindowMessage::Hide));
            }
            SystemCommandResponse::Allow
        }
        SystemCommand::Minimize => {
            if state.store.settings.minimize_to_tray_on_minimize {
                return SystemCommandResponse::PreventWith(RootMessage::Window(WindowMessage::Hide));
            }
            SystemCommandResponse::Allow
        }
        SystemCommand::Maximize | SystemCommand::Restore => SystemCommandResponse::Allow,
        _ => SystemCommandResponse::Allow,
    })
    .replace_titlebar()
    .with_backdrop(Backdrop::MicaAlt)
    .with_window_size(960, 760)
    .with_initial_display(InitialDisplay::Hidden);

    app.run()?;

    Ok(())
}
