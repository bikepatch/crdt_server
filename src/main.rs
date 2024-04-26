use futures_util::{sink::SinkExt, stream::StreamExt, stream::{SplitSink, SplitStream}};
use serde::{Deserialize, Serialize};
use serde_json::Error;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::{accept_async, tungstenite::protocol::Message, WebSocketStream};
use tokio::sync::mpsc;
use tokio::sync::mpsc::{UnboundedSender, UnboundedReceiver};
use tokio::signal;

// Operation to be tranmitted
#[derive(Serialize, Deserialize, Debug)]
struct CrdtOperation {
    player_id: String,
    action: String,
    op_id: String,
    timestamp: i64,  // Timestamp is needed for conflict resolution
}

// Here is a table of clients
type Client = UnboundedSender<Message>;
type Clients = Arc<Mutex<HashMap<String, Client>>>;

#[tokio::main]
async fn main() {
    let listener = TcpListener::bind("127.0.0.1:8080").await.expect("Failed to bind server");
    println!("Server listening on port 8080");

    let clients: Clients = Arc::new(Mutex::new(HashMap::new()));

    // Just to be able to shut down this whole thing
    let shutdown_signal = signal::ctrl_c();
    
    // Server
    let server = async move {
        while let Ok((stream, _)) = listener.accept().await {
            println!("Client connected");
            let ws_stream = accept_async(stream).await.expect("Error during the websocket handshake");
            let (write, read) = ws_stream.split();
            let (tx, rx) = mpsc::unbounded_channel();
            let clients_inner = clients.clone();

            let client_id = format!("Client_{}", generate_unique_id()); 
            clients.lock().unwrap().insert(client_id.clone(), tx);

            tokio::spawn(handle_messages(rx, write));
            tokio::spawn(read_messages(read, clients_inner, client_id));
        }
    };

    tokio::select! {
        _ = shutdown_signal => {
            println!("Ctrl+C signal received, shutting down.");
        },
        _ = server => {
            println!("Server task completed.");
        },
    }

    println!("Server is shutting down...");
}

async fn read_messages(mut read: SplitStream<WebSocketStream<TcpStream>>, clients: Clients, client_id: String) {
    while let Some(Ok(message)) = read.next().await {
        match message {
            Message::Text(text) => {
                println!("Received message from {}: {}", client_id, text);
                if let Ok(operation) = serde_json::from_str::<CrdtOperation>(&text) {
                    println!("Operation received: {:?}", operation);
                    send_operation(operation, &clients).await;
                } else {
                    println!("Failed to parse message into CrdtOperation: {}", text);
                }
            }
            _ => println!("Received non-text message or failed to convert message into text"),
        }
    }

    clients.lock().unwrap().remove(&client_id);
    println!("{} has disconnected", client_id);
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

// Id gen
fn generate_unique_id() -> String {
    use rand::{distributions::Alphanumeric, Rng};
    rand::thread_rng()
        .sample_iter(&Alphanumeric)
        .take(7)
        .map(char::from)
        .collect()
}