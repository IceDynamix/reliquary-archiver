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
//! - [`settings`]: User settings and their persistence
//! - [`messages`]: Message types for event handling
//! - [`handlers`]: Message handler implementations
//! - [`view`]: Main view rendering function
//! - [`theme`]: UI theming constants and helpers
//! - [`components`]: Reusable UI components (settings modal, log view, etc.)
//! - [`screens`]: Screen-specific views (waiting, active)
//! - [`kit`]: Low-level UI building blocks (icons, modals, toggles, tooltips)
//! - [`run_on_start`]: Windows startup registry integration
//! - [`worker`]: Background task running the packet capture

// TODO: the GUI only builds on Windows, remove this once its unused code has been cleaned up there
#![allow(unused)]

use async_stream::stream;
use raxis::runtime::Backdrop;
use raxis::runtime::task::{self, Task};
use raxis::runtime::window::builder::InitialDisplay;
use raxis::{ContextMenuItem, SystemCommand, SystemCommandResponse, TrayEvent, TrayIconConfig};
use tokio::sync::watch;
use tokio_stream::wrappers::WatchStream;

use crate::capture::Tap;
use crate::rgui::messages::{RootMessage, SettingsMessage, WebSocketStatus, WindowMessage};
use crate::rgui::state::RootState;
use crate::rgui::view::view;
use crate::rgui::worker::archiver_worker;
use crate::websocket::{PortCommand, PortSource, start_websocket_server};
use crate::{Args, logging};

// Module declarations
pub mod components;
pub mod handlers;
pub mod kit;
pub mod messages;
pub mod run_on_start;
mod scopefns;
pub mod screens;
pub mod settings;
pub mod state;
pub mod theme;
pub mod view;
pub mod worker;

/// Main update function that processes messages and modifies application state.
///
/// This is the core of the Elm architecture - all state changes flow through here.
/// Messages are dispatched to appropriate handlers based on their type.
///
/// Changed settings are saved afterwards, so handlers only have to change them.
///
/// # Returns
/// An optional [`Task`] to be executed asynchronously (e.g., I/O operations, timers).
pub fn update(state: &mut RootState, message: RootMessage) -> Option<Task<RootMessage>> {
    // The opacity slider changes the settings on every step of a drag, so its steps are saved once it
    // is let go. Only the steps are held back: the slider can miss being let go when the window loses
    // the mouse, and that mustn't keep everything else from being saved
    let hold_save = state.opacity_slider_dragging && matches!(message, RootMessage::Settings(SettingsMessage::OpacityChanged(_)));

    let task = route(state, message);

    if !hold_save {
        state.settings_file.save_if_changed(&state.store.settings);
    }

    task
}

/// Dispatches messages to their handlers
fn route(state: &mut RootState, message: RootMessage) -> Option<Task<RootMessage>> {
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
        RootMessage::WorkerEvent(event) => Some(handlers::handle_worker_event(state, event)),
        RootMessage::CheckConnection(now) => handlers::handle_connection_check(state, now),

        // Grouped message handlers - delegate to appropriate modules
        RootMessage::Account(msg) => handlers::handle_account_message(state, msg),
        RootMessage::Export(msg) => handlers::handle_export_message(state, msg),
        RootMessage::WebSocket(msg) => handlers::handle_websocket_message(state, msg),
        RootMessage::Settings(msg) => {
            components::settings_modal::handle_settings_message(state, msg);
            None
        }
        RootMessage::Window(msg) => handlers::handle_window_message(state, msg),
        RootMessage::Log(msg) => handlers::handle_log_message(state, msg),
        RootMessage::Update(msg) => handlers::handle_update_message(state, msg),
    }
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
pub fn run(tap: Tap) -> Result<(), Box<dyn std::error::Error>> {
    use std::sync::Arc;

    use futures::lock::Mutex;
    use reliquary_archiver::export::database::get_database;

    use crate::accounts::MultiAccountManager;
    use crate::rgui::components::update;

    let (settings, settings_file) = settings::load();
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

    let state = RootState::new(manager.clone(), port_tx, settings, settings_file);

    let app = raxis::Application::new(state, view, update, move |state| {
        let selected_account_rx = state.selected_account_tx.subscribe();

        Some(Task::batch(vec![
            Task::future(state.settings_file.writer()).discard(),
            Task::done(RootMessage::Update(update::UpdateMessage::PerformCheck)),
            Task::run(archiver_worker(manager.clone(), tap.clone()), RootMessage::WorkerEvent),
            Task::future(start_websocket_server(
                PortSource::Dynamic(WatchStream::new(port_rx.clone())),
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
                    logging::wait_for_new_lines().await;
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
