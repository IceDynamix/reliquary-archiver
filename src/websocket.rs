use std::error::Error;
use std::fmt::{self, Display};
use std::io;
use std::sync::Arc;

use axum::extract::ws::{Message, Utf8Bytes, WebSocket, WebSocketUpgrade};
use axum::routing::get;
use axum::{Extension, Router};
use futures::lock::Mutex;
use futures::sink::SinkExt;
use futures::stream::{SplitSink, StreamExt};
use reliquary_archiver::export::Exporter;
use reliquary_archiver::export::fribbels::OptimizerEvent;
use serde::Serialize;
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::watch;
use tokio::task::AbortHandle;
use tracing::{error, info, warn};

use crate::accounts::{AccountEvent, MultiAccountManager};

/// Why the server could not start
#[derive(Debug, Clone)]
pub enum ServerError {
    /// Something else listens on the port. Only the GUI reads it, the CLI knows its one port
    #[cfg_attr(not(feature = "gui"), allow(dead_code))]
    PortInUse(u16),
    Other(String),
}

impl Display for ServerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ServerError::PortInUse(_) => {
                f.write_str("Address already in use, please close any other instances of the application or try another address")
            }
            ServerError::Other(e) => f.write_str(e),
        }
    }
}

impl Error for ServerError {}

struct WebSocketServerState {
    manager: Arc<Mutex<MultiAccountManager>>,
    selected_account_rx: watch::Receiver<Option<u32>>,
    /// How many clients are connected
    client_count: watch::Sender<usize>,
}

impl WebSocketServerState {
    /// Sends the complete state of the account to the client, if there is one to send yet
    async fn send_initial_scan(&self, uid: u32, sender: &mut SplitSink<WebSocket, Message>) -> Result<(), Box<dyn Error>> {
        let export = match self.manager.lock().await.account_exporter(uid) {
            Some(exporter) if exporter.is_initialized() => exporter.export(),
            _ => None,
        };

        match export {
            Some(export) => send_serialized_message(sender, OptimizerEvent::InitialScan(export)).await,
            None => Ok(()),
        }
    }

    /// Changes the client count by `change` and returns the new count
    fn count_clients(&self, change: isize) -> usize {
        let mut count = 0;
        self.client_count.send_modify(|clients| {
            *clients = clients.saturating_add_signed(change);
            count = *clients;
        });
        count
    }
}

#[cfg(feature = "gui")]
#[derive(Clone)]
pub enum PortCommand {
    Open(u16),
    Close,
}

/// Serves on `port` for as long as the program runs
pub async fn start_websocket_server(
    port: u16,
    manager: Arc<Mutex<MultiAccountManager>>,
    selected_account_rx: watch::Receiver<Option<u32>>,
) -> Result<(), ServerError> {
    let state = Arc::new(WebSocketServerState {
        manager,
        selected_account_rx,
        client_count: watch::channel(0).0,
    });

    serve(port, &state).await.map(|_| ())
}

/// Serves on the port `commands` asked for last, until it asks to close.
///
/// Returns the ports the server opens (or why it couldn't), and how many clients are connected.
/// A server that runs keeps running when it can't move to another port.
#[cfg(feature = "gui")]
pub fn start_dynamic_websocket_server(
    mut commands: tokio_stream::wrappers::WatchStream<PortCommand>,
    manager: Arc<Mutex<MultiAccountManager>>,
    selected_account_rx: watch::Receiver<Option<u32>>,
) -> (
    impl futures::Stream<Item = Result<u16, ServerError>>,
    impl futures::Stream<Item = usize>,
) {
    use tokio_stream::wrappers::WatchStream;

    let (client_count, client_count_rx) = watch::channel(0);
    let (port_tx, port_rx) = watch::channel::<Result<u16, ServerError>>(Ok(0));

    let state = Arc::new(WebSocketServerState {
        manager,
        selected_account_rx,
        client_count,
    });

    tokio::spawn(async move {
        let mut running: Option<(u16, AbortHandle)> = None;

        while let Some(command) = commands.next().await {
            match command {
                PortCommand::Close => stop(&mut running),
                PortCommand::Open(port) => match serve(port, &state).await {
                    Ok(server) => {
                        stop(&mut running);
                        port_tx.send(Ok(server.0)).ok();
                        running = Some(server);
                    }
                    Err(e) => {
                        port_tx.send(Err(e)).ok();
                    }
                },
            }
        }
    });

    // The port channel starts out with a placeholder, only report the ports the server actually runs on
    (WatchStream::from_changes(port_rx), WatchStream::new(client_count_rx))
}

#[cfg(feature = "gui")]
fn stop(running: &mut Option<(u16, AbortHandle)>) {
    if let Some((port, handle)) = running.take() {
        info!("Terminating WebSocket server on 0.0.0.0:{}", port);
        handle.abort();
    }
}

/// Starts serving on `port` (any free one for 0). Returns the port and what stops the server.
async fn serve(port: u16, state: &Arc<WebSocketServerState>) -> Result<(u16, AbortHandle), ServerError> {
    let addr = std::net::SocketAddr::from(([0, 0, 0, 0], port));

    let listener = tokio::net::TcpListener::bind(addr).await.map_err(|e| match e.kind() {
        io::ErrorKind::AddrInUse => ServerError::PortInUse(port),
        _ => ServerError::Other(e.to_string()),
    })?;
    let local_addr = listener.local_addr().map_err(|e| ServerError::Other(e.to_string()))?;

    let service = Router::new()
        .route("/ws", get(ws_handler))
        .layer(Extension(state.clone()))
        .into_make_service();

    let task = tokio::spawn(async move {
        info!("Starting WebSocket server on {}", local_addr);
        if let Err(e) = axum::serve(listener, service).await {
            error!("WebSocket server on {} stopped: {}", local_addr, e);
        }
    });

    Ok((local_addr.port(), task.abort_handle()))
}

async fn ws_handler(ws: WebSocketUpgrade, Extension(state): Extension<Arc<WebSocketServerState>>) -> axum::response::Response {
    ws.on_upgrade(|socket| handle_socket(socket, state))
}

async fn send_serialized_message<T: Serialize>(sender: &mut SplitSink<WebSocket, Message>, message: T) -> Result<(), Box<dyn Error>> {
    let message = serde_json::to_string(&message)?;
    sender.send(Message::Text(Utf8Bytes::from(message))).await?;

    Ok(())
}

async fn handle_socket(socket: WebSocket, state: Arc<WebSocketServerState>) {
    let client_count = state.count_clients(1);
    info!("New client connected, total clients: {}", client_count);

    let (mut sender, mut receiver) = socket.split();

    // Subscribe to manager's unified event channel
    let mut manager_rx = state.manager.lock().await.subscribe();
    let mut account_rx = state.selected_account_rx.clone();

    // Sync with GUI's current selection
    let mut selected_uid = *account_rx.borrow();

    // Send initial state if an account is already selected
    if let Some(uid) = selected_uid
        && let Err(e) = state.send_initial_scan(uid, &mut sender).await
    {
        warn!("Failed to send initial state: {}", e);
    }

    // Main loop: filter events by selected account and track GUI selection changes
    loop {
        tokio::select! {
            // Events from manager
            event = manager_rx.recv() => {
                match event {
                    Ok(AccountEvent::ExporterEvent { uid, event }) => {
                        // Only send if this is the selected account
                        if Some(uid) == selected_uid
                            && let Err(e) = send_serialized_message(&mut sender, event).await
                        {
                            warn!("Failed to send event to client: {}", e);
                            break;
                        }
                    }
                    Ok(AccountEvent::Discovered { .. } | AccountEvent::Reconnected { .. }) => {
                        // No-op, if the account is selected, the exporter will send the initial state on selection change.
                        // Or for the automatic initial export after exporter initialization.
                    }
                    // The client missed updates, the whole state brings it up to date again
                    Err(RecvError::Lagged(skipped)) => {
                        warn!(skipped, "WebSocket client fell behind, sending the whole state again");
                        if let Some(uid) = selected_uid
                            && let Err(e) = state.send_initial_scan(uid, &mut sender).await
                        {
                            warn!("Failed to send initial state after falling behind: {}", e);
                            break;
                        }
                    }
                    Err(RecvError::Closed) => break,
                }
            }
            // GUI selection changed
            result = account_rx.changed() => {
                if result.is_err() {
                    break;
                }
                let new_uid = *account_rx.borrow_and_update();
                if new_uid != selected_uid {
                    selected_uid = new_uid;
                    info!("WebSocket switched to account {:?}", selected_uid);

                    // Send initial state for newly selected account
                    if let Some(uid) = selected_uid
                        && let Err(e) = state.send_initial_scan(uid, &mut sender).await
                    {
                        warn!("Failed to send initial state after selection: {}", e);
                        break;
                    }
                }
            }
            // Client messages (for future account selection or keepalive)
            msg = receiver.next() => {
                if msg.is_none() {
                    // Client disconnected
                    info!("WebSocket client disconnected");
                    break;
                }
            }
        }
    }

    let client_count = state.count_clients(-1);
    info!("Client disconnected, total clients: {}", client_count);
}
