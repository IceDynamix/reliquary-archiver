use std::sync::Arc;
use axum::{
    extract::ws::{Message, Utf8Bytes, WebSocket, WebSocketUpgrade},
    routing::any,
    Extension, Router
};
use tokio::sync::broadcast;
use tracing::{debug, info, warn};
use serde::Serialize;

//allows to split the websocket stream into separate TX and RX branches
use futures::{sink::SinkExt, stream::StreamExt};

pub type ClientSender = broadcast::Sender<String>;

pub struct WebSocketState {
    pub tx: ClientSender,
}

pub async fn start_websocket_server(
    port: u16,
) -> (ClientSender, tokio::task::JoinHandle<()>) {
    // Create a broadcast channel with a reasonable capacity
    let (tx, _) = broadcast::channel::<String>(100);
    let state = Arc::new(WebSocketState { tx: tx.clone() });

    let app = Router::new()
        .route("/ws", any(ws_handler))
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

async fn ws_handler(
    ws: WebSocketUpgrade,
    Extension(state): Extension<Arc<WebSocketState>>,
) -> axum::response::Response {
    ws.on_upgrade(|socket| handle_socket(socket, state))
}

async fn handle_socket(
    socket: WebSocket,
    state: Arc<WebSocketState>,
) {
    let (mut sender, mut receiver) = socket.split();
    let mut rx = state.tx.subscribe();

    // Forward messages from the channel to the websocket
    let mut send_task = tokio::spawn(async move {
        while let Ok(msg) = rx.recv().await {
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
pub fn broadcast_message(
    tx: &ClientSender, 
    message: String
) {
    // It's okay if there are no receivers
    let _ = tx.send(message);
}

// Helper to create an update payload with a specific type
#[derive(Serialize, Clone)]
pub struct Update<T> {
    pub event: &'static str,
    pub data: T,
}

pub fn create_update<T: Serialize + Clone>(event: &'static str, data: T) -> String {
    let update = Update { event, data };
    serde_json::to_string(&update).unwrap_or_default()
} 