use std::error::Error;
use tokio::io::{self, AsyncBufReadExt, BufReader};
use tracing::{info, error};
use vstp::{tcp::VstpTcpClient, types::FrameType};

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    // Initialize tracing
    tracing_subscriber::fmt::init();

    info!("Starting Interactive VSTP TCP Client...");
    println!("Connecting to server at 127.0.0.1:6969...");

    // Connect to the server
    let mut client = match VstpTcpClient::connect("127.0.0.1:6969").await {
        Ok(c) => c,
        Err(e) => {
            error!("Failed to connect: {}", e);
            return Err(e.into());
        }
    };

    // Send HELLO to start the session
    client.send_hello().await?;
    println!("CONNECTED! You can start typing messages now.");
    println!("Type 'exit' to quit.");

    let (mut stdin_reader, mut client_recv) = (
        BufReader::new(io::stdin()).lines(),
        client
    );

    // Main loop
    loop {
        tokio::select! {
            // Handle user input from terminal
            line_res = stdin_reader.next_line() => {
                match line_res {
                    Ok(Some(line)) => {
                        let trimmed = line.trim();
                        if trimmed == "exit" {
                            println!("Closing connection...");
                            client_recv.close().await?;
                            break;
                        }
                        
                        if !trimmed.is_empty() {
                            client_recv.send_data(trimmed.as_bytes().to_vec()).await?;
                        }
                    }
                    Ok(None) => break, // EOF
                    Err(e) => {
                        error!("Error reading stdin: {}", e);
                        break;
                    }
                }
            }
            // Handle incoming frames from server
            frame_res = client_recv.recv() => {
                match frame_res {
                    Ok(Some(frame)) => {
                        match frame.typ {
                            FrameType::Data => {
                                if let Ok(payload_str) = String::from_utf8(frame.payload) {
                                    println!("\n>> Server: {}", payload_str);
                                }
                            }
                            FrameType::Bye => {
                                println!("\n>> Server closed connection.");
                                break;
                            }
                            _ => {
                                info!("Received frame: {:?}", frame.typ);
                            }
                        }
                    }
                    Ok(None) => {
                        println!("\n>> Connection lost.");
                        break;
                    }
                    Err(e) => {
                        error!("Error receiving from server: {}", e);
                        break;
                    }
                }
            }
        }
    }

    info!("Client example completed.");
    Ok(())
}
