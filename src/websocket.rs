use std::sync::{Arc, Mutex};
use axum::{
    extract::ws::{Message, Utf8Bytes, WebSocket, WebSocketUpgrade},
    routing::get,
    Extension, Router
};
use tokio::sync::broadcast;
use tracing::{debug, info, warn};
use serde::Serialize;

//allows to split the websocket stream into separate TX and RX branches
use futures::{sink::SinkExt, stream::StreamExt};

use crate::export::Exporter;

pub type ClientSender = broadcast::Sender<String>;

pub struct ApplicationState<E: Exporter> {
    pub tx: ClientSender,
    pub exporter: Arc<Mutex<E>>,
}

pub async fn start_websocket_server<E: Exporter>(
    port: u16,
    exporter: Arc<Mutex<E>>,
) -> (ClientSender, tokio::task::JoinHandle<()>) {
    // Create a broadcast channel with a reasonable capacity
    let (tx, _) = broadcast::channel::<String>(100);
    
    // Create the state
    let state = Arc::new(ApplicationState {
        tx: tx.clone(),
        exporter: exporter.clone(),
    });

    let app = Router::new()
        .route("/ws", get(ws_handler::<E>))
        .layer(Extension(state));

    let server_addr = format!("0.0.0.0:{}", port);
    info!("Starting WebSocket server on {}", server_addr);

    let handle = tokio::spawn(async move {
        let listener = tokio::net::TcpListener::bind(server_addr.parse::<std::net::SocketAddr>().unwrap()).await.unwrap();
        debug!("Listening on {}", listener.local_addr().unwrap());
        if let Err(e) = axum::serve(listener, app.into_make_service()).await 
        {
            warn!("Server error: {}", e);
        }
    });

    (tx, handle)
}

async fn ws_handler<E: Exporter>(
    ws: WebSocketUpgrade,
    Extension(state): Extension<Arc<ApplicationState<E>>>,
) -> axum::response::Response {
    ws.on_upgrade(|socket| handle_socket(socket, state))
}

async fn handle_socket<E: Exporter>(
    socket: WebSocket,
    state: Arc<ApplicationState<E>>,
) {
    info!("New client connected");
    let (mut sender, mut receiver) = socket.split();

    // Send the initial exporter state to the client
    {
        let message = {
            let exporter = state.exporter.lock().unwrap();

            exporter
                .is_finished()
                .then(|| exporter.get_initial_event())
                .flatten()
                .map(|e| serde_json::to_string(&e).unwrap())
        };

        if let Some(message) = message {
            if let Err(e) = sender.send(Message::Text(Utf8Bytes::from(message))).await {
                warn!("Failed to send exporter state to client: {}", e);
            }
        }
    }
    
    // Subscribe to new messages
    let mut rx = state.tx.subscribe();

    // Forward messages from the channel to the websocket
    let mut send_task = tokio::spawn(async move {
        while let Ok(msg) = rx.recv().await {
            // Forward to client
            if let Err(e) = sender.send(Message::Text(Utf8Bytes::from(msg))).await {
                warn!("Failed to send message to client: {}", e);
                break;
            }
        }
    });

    // Close if the client disconnects
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
}

// Helper function to broadcast an update to all connected clients
pub fn broadcast_message<T: Serialize>(
    client: &ClientSender, 
    message: T
) {
    // Broadcast to all connected clients
    let message = serde_json::to_string(&message).unwrap_or_default();
    if client.send(message).is_err() {
        warn!("No connected clients! Please turn on 'Live Imports' in the Optimizer Import tab.");
    }
}
