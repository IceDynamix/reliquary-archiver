use axum::{
    extract::ws::{Message, Utf8Bytes, WebSocket, WebSocketUpgrade},
    routing::get,
    Extension, Router,
};
use futures::{
    lock::Mutex,
    sink::SinkExt,
    stream::{SplitSink, Stream, StreamExt},
};
use reliquary_archiver::export::Exporter;
use serde::Serialize;
use std::{
    error::Error,
    io,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
};
use tokio::sync::watch;
use tracing::{debug, info, warn};

struct WebSocketServerState<E: Exporter> {
    pub exporter: Arc<Mutex<E>>,
    pub client_count: Arc<AtomicUsize>,
    pub client_count_tx: watch::Sender<usize>,
}

pub async fn start_websocket_server<E: Exporter>(port: u16, exporter: Arc<Mutex<E>>) -> Result<(u16, impl Stream<Item = usize>), String> {
    let (client_count_tx, client_count_rx) = watch::channel(0);
    let state = Arc::new(WebSocketServerState {
        exporter: exporter.clone(),
        client_count: Arc::new(AtomicUsize::new(0)),
        client_count_tx,
    });

    let app = Router::new().route("/ws", get(ws_handler::<E>)).layer(Extension(state));

    let server_addr = format!("0.0.0.0:{}", port);
    info!("Starting WebSocket server on {}", server_addr);

    let listener = tokio::net::TcpListener::bind(server_addr.parse::<std::net::SocketAddr>().unwrap())
        .await
        .map_err(|e| match e.kind() {
            io::ErrorKind::AddrInUse => "Address already in use, please close any other instances of the application".to_string(),
            _ => e.to_string(),
        })?;

    tokio::spawn(async move {
        debug!("Listening on {}", listener.local_addr().unwrap());
        axum::serve(listener, app.into_make_service()).await.unwrap();
    });

    // Create a stream from the watch receiver
    let client_count_stream = tokio_stream::wrappers::WatchStream::new(client_count_rx);

    Ok((port, client_count_stream))
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
