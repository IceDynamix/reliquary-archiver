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

use async_stream::stream;
use raxis::runtime::Backdrop;
use raxis::runtime::task::{self, Task};
use raxis::runtime::window::builder::InitialDisplay;
use raxis::{ContextMenuItem, SystemCommand, SystemCommandResponse, TrayEvent, TrayIconConfig};
use tokio::sync::watch;
use tokio_stream::wrappers::WatchStream;

use crate::LOG_NOTIFY;
use crate::rgui::components::update::UpdateMessage;
use crate::scopefns::Also;
use crate::websocket::{PortCommand, PortSource, start_websocket_server};
use crate::worker::archiver_worker;

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

/// Loads settings synchronously from disk before the GUI initializes.
///
/// Uses the new `raxis::get_local_app_data()` synchronous helper to resolve the
/// settings path without needing an async task, preventing race conditions between
/// settings loading and other startup tasks (update checks, WebSocket port binding).
///
/// Returns the loaded settings and a bool indicating whether the `run_on_start`
/// value was corrected during registry reconciliation (and therefore needs to be
/// persisted back to disk).
fn load_settings_sync() -> (state::Settings, bool) {
    use crate::rgui::run_on_start::registry_matches_settings;

    let Some(appdata) = raxis::get_local_app_data() else {
        tracing::warn!("Could not determine local app data path, using default settings");
        return (state::Settings::default(), false);
    };

    let path = get_settings_path(appdata);

    let content = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            tracing::info!("No settings file found at {}, using defaults", path.display());
            return (state::Settings::default(), false);
        }
        Err(e) => {
            tracing::error!("Failed to read settings file: {}", e);
            return (state::Settings::default(), false);
        }
    };

    let mut settings: state::Settings = match serde_json::from_str(&content) {
        Ok(s) => s,
        Err(e) => {
            tracing::error!("Failed to parse settings file: {}", e);
            return (state::Settings::default(), false);
        }
    };

    // Reconcile run_on_start setting with registry state.
    // e.g. user moves the exe after enabling/disabling run on start — in case
    // of mismatch, update settings in memory and signal that a save is needed.
    let run_on_start = settings.run_on_start;
    let needs_save = match registry_matches_settings(run_on_start) {
        Ok(false) => {
            settings.run_on_start = !run_on_start;
            true
        }
        Ok(true) | Err(_) => false,
    };

    tracing::info!("Loaded settings from {}", path.display());
    (settings, needs_save)
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

    let (settings, settings_needs_save) = load_settings_sync();
    let start_minimized = settings.start_minimized;
    let minimize_to_tray_on_minimize = settings.minimize_to_tray_on_minimize;

    // Determine the initial window display state up front so that the window
    // is shown/hidden/minimized at creation time rather than via a deferred
    // task, avoiding a brief invisible-window period at startup.
    let initial_display = if start_minimized {
        if minimize_to_tray_on_minimize {
            // Hidden to tray — window should not appear on the taskbar at all.
            InitialDisplay::Hidden
        } else {
            // Minimized to taskbar.
            InitialDisplay::Minimized
        }
    } else {
        InitialDisplay::Shown
    };

    // Initialize the port channel with the correct port from settings so the
    // WebSocket server binds the right port immediately — no SendPort message needed.
    let (port_tx, port_rx) = watch::channel::<PortCommand>(PortCommand::Open(settings.ws_port));

    let database = get_database();
    let manager = Arc::new(Mutex::new(MultiAccountManager::new()));

    let state = RootState::new(manager.clone()).with_port_sender(port_tx).with_settings(settings);

    let app = raxis::Application::new(state, view, update, move |state| {
        let selected_account_rx = state.selected_account_tx.subscribe();

        // Persist corrected run_on_start value if the registry was out of sync.
        let save_task = if settings_needs_save {
            Task::done(RootMessage::Settings(SettingsMessage::Save))
        } else {
            Task::none()
        };

        Some(Task::batch(vec![
            save_task,
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
    .with_initial_display(initial_display);

    app.run()?;

    Ok(())
}
