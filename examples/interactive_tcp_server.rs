use std::collections::HashMap;
use std::error::Error;
use std::sync::Arc;
use tokio::io::{self, AsyncBufReadExt, BufReader};
use tokio::sync::{broadcast, Mutex};
use tracing::{info, error, debug};
use vstp::{
    tcp::VstpTcpServer,
    types::{Frame, FrameType, SessionId},
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    // Initialize tracing
    tracing_subscriber::fmt::init();

    info!("Starting Interactive VSTP TCP Server...");
    println!("Listening on 127.0.0.1:6969...");

    // Bind to default port
    let server = VstpTcpServer::bind("127.0.0.1:6969").await?;

    // Map to keep track of active connections for broadcasting terminal input
    let connections: Arc<Mutex<HashMap<SessionId, tokio::sync::mpsc::UnboundedSender<Frame>>>> = 
        Arc::new(Mutex::new(HashMap::new()));

    // Channel for broadcasting terminal input to all connected clients
    let (tx, _rx) = broadcast::channel::<String>(100);

    // Terminal input task
    let connections_clone = connections.clone();
    let tx_clone = tx.clone();
    tokio::spawn(async move {
        let mut stdin_reader = BufReader::new(io::stdin()).lines();
        println!("Server Console: Type something to broadcast to all clients.");
        
        while let Ok(Some(line)) = stdin_reader.next_line().await {
            let trimmed = line.trim();
            if !trimmed.is_empty() {
                if let Err(e) = tx_clone.send(trimmed.to_string()) {
                    debug!("Broadcast error (no clients?): {}", e);
                }
            }
        }
    });

    // Main accept loop
    loop {
        match server.accept().await {
            Ok(mut conn) => {
                let session_id = conn.session_id();
                
                let (msg_tx, mut msg_rx) = tokio::sync::mpsc::unbounded_channel::<Frame>();
                
                // Add to active connections
                {
                    let mut conns = connections_clone.lock().await;
                    conns.insert(session_id, msg_tx);
                }

                let tx_sub = tx.subscribe();
                let connections_task_clone = connections_clone.clone();

                tokio::spawn(async move {
                    info!("Client (Session {}) joined.", session_id);
                    println!("\n>> Client {} connected.", session_id);

                    let mut broadcast_rx = tx_sub;

                    loop {
                        tokio::select! {
                            // Receive from client
                            frame_res = conn.recv() => {
                                match frame_res {
                                    Ok(Some(frame)) => {
                                        match frame.typ {
                                            FrameType::Data => {
                                                if let Ok(payload_str) = String::from_utf8(frame.payload) {
                                                    println!("\n>> Client {}: {}", session_id, payload_str);
                                                }
                                            }
                                            FrameType::Bye => {
                                                println!("\n>> Client {} disconnected.", session_id);
                                                break;
                                            }
                                            _ => {
                                                debug!("Received frame: {:?}", frame.typ);
                                            }
                                        }
                                    }
                                    Ok(None) => break,
                                    Err(e) => {
                                        error!("Error receiving from client {}: {}", session_id, e);
                                        break;
                                    }
                                }
                            }
                            // Receive broadcast from terminal
                            broadcast_res = broadcast_rx.recv() => {
                                if let Ok(msg) = broadcast_res {
                                    let data_frame = Frame::new(FrameType::Data).with_payload(msg.as_bytes().to_vec());
                                    if let Err(e) = conn.send(data_frame).await {
                                        error!("Error sending to client {}: {}", session_id, e);
                                        break;
                                    }
                                }
                            }
                            // Local messages to send
                            msg_res = msg_rx.recv() => {
                                if let Some(frame) = msg_res {
                                    if let Err(e) = conn.send(frame).await {
                                        error!("Error sending internal msg to client {}: {}", session_id, e);
                                        break;
                                    }
                                }
                            }
                        }
                    }

                    // Remove from active connections
                    {
                        let mut conns = connections_task_clone.lock().await;
                        conns.remove(&session_id);
                    }
                    info!("Client (Session {}) disconnected.", session_id);
                });
            }
            Err(e) => {
                error!("Failed to accept connection: {}", e);
            }
        }
    }
}
