use std::sync::Arc;
use axum::{
    extract::ws::{Message, Utf8Bytes, WebSocket, WebSocketUpgrade},
    routing::get,
    Extension, Router
};
use tokio::sync::{broadcast, Mutex};
use tracing::{debug, info, warn};
use serde::Serialize;

//allows to split the websocket stream into separate TX and RX branches
use futures::{sink::SinkExt, stream::StreamExt};

// Create a type that includes both the sender and message history
pub struct ClientSender {
    pub tx: broadcast::Sender<String>,
    pub message_history: Arc<Mutex<Vec<String>>>,
}

pub struct WebSocketState {
    pub client: ClientSender,
}

pub async fn start_websocket_server(
    port: u16,
) -> (ClientSender, tokio::task::JoinHandle<()>) {
    // Create a broadcast channel with a reasonable capacity
    let (tx, _) = broadcast::channel::<String>(100);
    
    // Create the client with history
    let client = ClientSender {
        tx: tx.clone(),
        message_history: Arc::new(Mutex::new(Vec::new())),
    };
    
    // Create the state
    let state = Arc::new(WebSocketState {
        client: ClientSender {
            tx: tx.clone(),
            message_history: client.message_history.clone(),
        }
    });

    {
        let mut rx = state.client.tx.subscribe();
        let message_history = state.client.message_history.clone();
        tokio::spawn(async move {
            while let Ok(msg) = rx.recv().await {
                // Add message to history
                {
                    let mut history = message_history.lock().await;
                    history.push(msg.clone());
                }
            }
        });
    }

    let app = Router::new()
        .route("/ws", get(ws_handler))
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

    (client, handle)
}

async fn ws_handler(
    ws: WebSocketUpgrade,
    Extension(state): Extension<Arc<WebSocketState>>,
) -> axum::response::Response {
    info!("New client connected");
    ws.on_upgrade(|socket| handle_socket(socket, state))
}

async fn handle_socket(
    socket: WebSocket,
    state: Arc<WebSocketState>,
) {
    info!("New socket connected");
    let (mut sender, mut receiver) = socket.split();
    
    // Send message history to the new client first
    {
        let history = state.client.message_history.lock().await;
        for msg in history.iter() {
            if let Err(e) = sender.send(Message::Text(Utf8Bytes::from(msg.clone()))).await {
                warn!("Failed to send history message to client: {}", e);
                return;
            }
        }
        debug!("Sent {} history messages to new client", history.len());
    }
    
    // Subscribe to new messages
    let mut rx = state.client.tx.subscribe();

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
    // Message will be added to history by the receiving tasks
    let message = serde_json::to_string(&message).unwrap_or_default();
    let _ = client.tx.send(message);
}
