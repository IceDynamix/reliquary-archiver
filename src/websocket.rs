use std::error::Error;
use std::io;
use std::sync::atomic::{AtomicU16, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex as StdMutex};

use axum::extract::ws::{Message, Utf8Bytes, WebSocket, WebSocketUpgrade};
use axum::routing::get;
use axum::{Extension, Router};
use futures::lock::Mutex;
use futures::sink::SinkExt;
use futures::stream::{SplitSink, Stream, StreamExt};
use reliquary_archiver::export::Exporter;
use reliquary_archiver::export::fribbels::OptimizerEvent;
use serde::Serialize;
use tokio::sync::watch;
use tokio::task::AbortHandle;
use tokio_stream::wrappers::WatchStream;
use tracing::{debug, info, warn};

use crate::accounts::{AccountEvent, MultiAccountManager};

struct WebSocketServerState {
    pub manager: Arc<Mutex<MultiAccountManager>>,
    pub selected_account_rx: watch::Receiver<Option<u32>>,
    pub client_count: Arc<AtomicUsize>,
    pub client_count_tx: watch::Sender<usize>,
    pub service_handle: StdMutex<Option<AbortHandle>>,
    pub active_port: AtomicU16,
}

impl WebSocketServerState {
    /// Replaces the running server (if any) with a new one
    pub fn replace_service(&self, handle: AbortHandle, port: u16) {
        self.abort_service();
        self.active_port.store(port, Ordering::Relaxed);
        *self.service_handle.lock().unwrap() = Some(handle);
    }

    pub fn abort_service(&self) {
        if let Some(handle) = self.service_handle.lock().unwrap().take() {
            let current_port = self.active_port.load(Ordering::Relaxed);
            info!("Terminating WebSocket server on 0.0.0.0:{}", current_port);
            handle.abort();
        }
    }

    /// Sends the complete state of the account to the client, if there is one to send yet
    async fn send_initial_scan(&self, uid: u32, sender: &mut SplitSink<WebSocket, Message>) -> Result<(), Box<dyn Error>> {
        let Some(exporter) = self.manager.lock().await.get_account_exporter(uid) else {
            return Ok(());
        };

        let export = {
            let exporter = exporter.lock().await;
            if !exporter.is_initialized() {
                return Ok(());
            }
            exporter.export()
        };

        match export {
            Some(export) => send_serialized_message(sender, OptimizerEvent::InitialScan(export)).await,
            None => Ok(()),
        }
    }
}

pub enum PortSource {
    Fixed(u16),
    #[cfg(feature = "gui")]
    Dynamic(WatchStream<PortCommand>),
}

#[cfg(feature = "gui")]
#[derive(Clone)]
pub enum PortCommand {
    Open(u16),
    Close,
}

pub async fn start_websocket_server(
    #[cfg_attr(not(feature = "gui"), allow(unused_mut))] mut port_source: PortSource,
    manager: Arc<Mutex<MultiAccountManager>>,
    selected_account_rx: watch::Receiver<Option<u32>>,
) -> Result<(impl Stream<Item = Result<u16, String>>, impl Stream<Item = usize>), String> {
    let initial_port = match port_source {
        PortSource::Fixed(ref port) => *port,
        #[cfg(feature = "gui")]
        PortSource::Dynamic(_) => 0,
    };

    let (client_count_tx, client_count_rx) = watch::channel(0);
    let (port_tx, port_rx) = watch::channel::<Result<u16, String>>(Ok(initial_port));

    let state = Arc::new(WebSocketServerState {
        manager: manager.clone(),
        selected_account_rx,
        client_count: Arc::new(AtomicUsize::new(0)),
        client_count_tx,
        service_handle: StdMutex::new(None),
        active_port: AtomicU16::new(initial_port),
    });

    // Create streams from the watch receivers
    let client_count_stream = WatchStream::new(client_count_rx);
    let port_stream = WatchStream::new(port_rx);

    tokio::spawn(async move {
        loop {
            // Without the gui feature there is only a single kind of port source
            #[cfg_attr(not(feature = "gui"), allow(clippy::infallible_destructuring_match))]
            let port = match port_source {
                PortSource::Fixed(port) => port,
                #[cfg(feature = "gui")]
                PortSource::Dynamic(ref mut stream) => match stream.next().await {
                    None => break,
                    Some(PortCommand::Close) => {
                        state.abort_service();
                        continue;
                    }
                    Some(PortCommand::Open(port)) => port,
                },
            };

            let service = Router::new()
                .route("/ws", get(ws_handler))
                .layer(Extension(state.clone()))
                .into_make_service();

            let addr = std::net::SocketAddr::from(([0, 0, 0, 0], port));

            let listener = tokio::net::TcpListener::bind(addr).await.map_err(|e| match e.kind() {
                io::ErrorKind::AddrInUse => {
                    "Address already in use, please close any other instances of the application or try another address".to_string()
                }
                _ => e.to_string(),
            });

            match listener {
                Ok(listener) => {
                    let port = listener.local_addr().unwrap().port();
                    state.replace_service(
                        tokio::spawn(async move {
                            let local_addr = listener.local_addr().unwrap();
                            info!("Starting WebSocket server on {}", local_addr);
                            debug!("Listening on {}", local_addr);
                            axum::serve(listener, service).await.unwrap();
                        })
                        .abort_handle(),
                        port,
                    );
                    port_tx.send(Ok(port)).ok();
                }
                Err(e) => {
                    port_tx.send(Err(e)).ok();
                }
            }

            if matches!(port_source, PortSource::Fixed(_)) {
                break;
            };
        }
    });

    Ok((port_stream, client_count_stream))
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
    // Increment client count
    let client_count = state.client_count.fetch_add(1, Ordering::SeqCst) + 1;
    info!("New client connected, total clients: {}", client_count);

    // Notify GUI of client count change
    let _ = state.client_count_tx.send(client_count);

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
            Ok(event) = manager_rx.recv() => {
                match event {
                    AccountEvent::ExporterEvent { uid, event } => {
                        // Only send if this is the selected account
                        if Some(uid) == selected_uid
                            && let Err(e) = send_serialized_message(&mut sender, event).await
                        {
                            warn!("Failed to send event to client: {}", e);
                            break;
                        }
                    }
                    AccountEvent::Discovered { .. } | AccountEvent::Reconnected { .. } => {
                        // No-op, if the account is selected, the exporter will send the initial state on selection change.
                        // Or for the automatic initial export after exporter initialization.
                    }
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

    // Decrement client count when client disconnects
    let client_count = state.client_count.fetch_sub(1, Ordering::SeqCst) - 1;
    info!("Client disconnected, total clients: {}", client_count);

    // Notify GUI of client count change
    let _ = state.client_count_tx.send(client_count);
}
