use futures_util::{sink::SinkExt, stream::{StreamExt, SplitStream, SplitSink}};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::{accept_async, tungstenite::protocol::Message, WebSocketStream};
use tokio::sync::mpsc;
use tokio::sync::mpsc::{UnboundedSender, UnboundedReceiver};
use tokio::signal;

// Operation to be transmitted
#[derive(Serialize, Deserialize, Debug)]
struct CrdtOperation {
    player_id: String,
    action: String,
    op_id: String,
    timestamp: i64,  // Timestamp is neede for future conflict
}

// Here s a table of clients
type Client = UnboundedSender<Message>;
type Clients = Arc<Mutex<HashMap<String, Client>>>;

#[tokio::main]
async fn main() {
    let listener = TcpListener::bind("127.0.0.1:17815").await.expect("Failed to bind server");
    println!("Server listening on port 17815");

    let clients: Clients = Arc::new(Mutex::new(HashMap::new()));

    // Just to be able to shut down this whole thing
    let shutdown_signal = signal::ctrl_c();

    // Server impl
    let server = async move {
        while let Ok((stream, _)) = listener.accept().await {
            let ws_stream = accept_async(stream).await.expect("WebSocket handshake error");
            let (write, read) = ws_stream.split();
            let (tx, rx) = mpsc::unbounded_channel();
            let clients_inner = clients.clone();

            tokio::spawn(handle_messages(rx, write));
            tokio::spawn(read_messages(read, tx, clients_inner));
        }
    };

    tokio::select! {
        _ = shutdown_signal => {
            println!("Ctrl+C signal received");
        },
        _ = server => {
            println!("Server task completed");
        },
    }

    println!("Server is shutting down...");
}

// Read what comes
async fn read_messages(mut read: SplitStream<WebSocketStream<TcpStream>>, tx: Client, clients: Clients) {
    while let Some(Ok(message)) = read.next().await {
        if let Ok(text) = message.into_text() {
            if let Ok(operation) = serde_json::from_str::<CrdtOperation>(&text) {
                send_operation(operation, &clients).await;
            }
        }
    }
}

// Handle what comes
async fn handle_messages(mut rx: UnboundedReceiver<Message>, mut write: SplitSink<WebSocketStream<TcpStream>, Message>) {
    while let Some(message) = rx.recv().await {
        write.send(message).await.expect("Failed to send message");
    }
}

// Send to others
async fn send_operation(operation: CrdtOperation, clients: &Clients) {
    let message = Message::Text(serde_json::to_string(&operation).unwrap());
    let clients = clients.lock().unwrap();

    for (_client_id, client) in clients.iter() {
        let _ = client.send(message.clone());
    }
}