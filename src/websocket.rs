use std::error::Error;
use std::io;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU16, AtomicUsize, Ordering};
use std::sync::{Arc, RwLock};

use axum::extract::ws::{Message, Utf8Bytes, WebSocket, WebSocketUpgrade};
use axum::routing::get;
use axum::{Extension, Router};
use futures::lock::Mutex;
use futures::sink::SinkExt;
use futures::stream::{SplitSink, Stream, StreamExt};
use reliquary_archiver::export::Exporter;
use serde::Serialize;
use tokio::sync::watch;
use tokio::task::AbortHandle;
use tokio_stream::wrappers::WatchStream;
use tracing::{debug, info, warn};

struct WebSocketServerState<E: Exporter> {
    pub exporter: Arc<Mutex<E>>,
    pub client_count: Arc<AtomicUsize>,
    pub client_count_tx: watch::Sender<usize>,
    pub service_handle: Arc<RwLock<Option<AbortHandle>>>,
    pub active_port: Arc<AtomicU16>,
}

impl<E: Exporter> WebSocketServerState<E> {
    pub fn set_service_handle(&self, handle: AbortHandle, port: u16) -> Result<(), ()> {
        self.active_port.store(port, Ordering::Relaxed);
        if let Ok(mut w) = self.service_handle.write() {
            *w = Some(handle);
            return Ok(());
        };
        Err(())
    }

    pub fn abort_service(&self) -> Result<(), ()> {
        if let Ok(r) = self.service_handle.read() {
            if let Some(handle) = r.as_ref() {
                let current_port = self.active_port.load(Ordering::Relaxed);
                info!("Terminating WebSocket server on 0.0.0.0:{}", current_port);
                handle.abort();
            }
            return Ok(());
        }
        Err(())
    }
}

pub enum PortSource {
    Fixed(u16),
    Dynamic(WatchStream<u16>),
}

pub async fn start_websocket_server<E: Exporter>(
    mut port_source: PortSource,
    exporter: Arc<Mutex<E>>,
) -> Result<(impl Stream<Item = Result<u16, String>>, impl Stream<Item = usize>), String> {
    let initial_port = match port_source {
        PortSource::Fixed(ref port) => *port,
        PortSource::Dynamic(_) => 0,
    };

    let (client_count_tx, client_count_rx) = watch::channel(0);
    let (port_tx, port_rx) = watch::channel::<Result<u16, String>>(Ok(initial_port));

    let state = Arc::new(WebSocketServerState {
        exporter: exporter.clone(),
        client_count: Arc::new(AtomicUsize::new(0)),
        client_count_tx,
        service_handle: Arc::new(RwLock::new(None)),
        active_port: Arc::new(AtomicU16::new(0)),
    });

    // Create streams from the watch receivers
    let client_count_stream = WatchStream::new(client_count_rx);
    let port_stream = WatchStream::new(port_rx);

    tokio::spawn(async move {
        loop {
            let port = match port_source {
                PortSource::Fixed(port) => port,
                PortSource::Dynamic(ref mut stream) => {
                    let new_port = stream.next().await;
                    if new_port.is_none() {
                        break;
                    }
                    new_port.unwrap()
                }
            };

            let server_addr = format!("0.0.0.0:{}", port);

            let service = Router::new()
                .route("/ws", get(ws_handler::<E>))
                .layer(Extension(state.clone()))
                .into_make_service();

            let addr = server_addr.parse::<std::net::SocketAddr>().unwrap();

            let listener = tokio::net::TcpListener::bind(addr).await.map_err(|e| match e.kind() {
                io::ErrorKind::AddrInUse => {
                    "Address already in use, please close any other instances of the application or try another address".to_string()
                }
                _ => e.to_string(),
            });

            match listener {
                Ok(listener) => {
                    let port = listener.local_addr().unwrap().port();
                    state.abort_service();
                    state.set_service_handle(
                        tokio::spawn(async move {
                            let local_addr = listener.local_addr().unwrap();
                            info!("Starting WebSocket server on {}", local_addr);
                            debug!("Listening on {}", local_addr);
                            axum::serve(listener, service).await.unwrap();
                        })
                        .abort_handle(),
                        port,
                    );
                    port_tx.send(Ok(port));
                }
                Err(e) => {
                    port_tx.send(Err(e));
                }
            }

            if matches!(port_source, PortSource::Fixed(_)) {
                break;
            };
        }
    });

    Ok((port_stream, client_count_stream))
}

async fn ws_handler<E: Exporter>(
    ws: WebSocketUpgrade,
    Extension(state): Extension<Arc<WebSocketServerState<E>>>,
) -> axum::response::Response {
    ws.on_upgrade(|socket| handle_socket(socket, state))
}

async fn send_serialized_message<T: Serialize>(sender: &mut SplitSink<WebSocket, Message>, message: T) -> Result<(), Box<dyn Error>> {
    let message = serde_json::to_string(&message)?;
    sender.send(Message::Text(Utf8Bytes::from(message))).await?;

    Ok(())
}

async fn handle_socket<E: Exporter>(socket: WebSocket, state: Arc<WebSocketServerState<E>>) {
    // Increment client count
    let client_count = state.client_count.fetch_add(1, Ordering::SeqCst) + 1;
    info!("New client connected, total clients: {}", client_count);

    // Notify GUI of client count change
    let _ = state.client_count_tx.send(client_count);

    let (mut sender, mut receiver) = socket.split();

    // Subscribe to the exporter's event channel
    let (initial_event, mut rx) = state.exporter.lock().await.subscribe();

    // Send the initial exporter state to the client
    if let Some(event) = initial_event {
        if let Err(e) = send_serialized_message(&mut sender, event).await {
            warn!("Failed to send initial state to client: {}", e);
        }
    }

    // Forward messages from the channel to the websocket
    let mut send_task = tokio::spawn(async move {
        while let Ok(msg) = rx.recv().await {
            if let Err(e) = send_serialized_message(&mut sender, msg).await {
                warn!("Failed to send event to client: {}", e);
                break;
            }
        }
    });

    let mut recv_task = tokio::spawn(async move {
        while let Some(Ok(_)) = receiver.next().await {
            // Just keep the connection alive, we don't need client messages
        }
    });

    // If any task exits, clean up both
    tokio::select! {
        _ = &mut send_task => {
            recv_task.abort();
        },
        _ = &mut recv_task => {
            send_task.abort();
        },
    }

    // Decrement client count when client disconnects
    let client_count = state.client_count.fetch_sub(1, Ordering::SeqCst) - 1;
    info!("Client disconnected, total clients: {}", client_count);

    // Notify GUI of client count change
    let _ = state.client_count_tx.send(client_count);
}
