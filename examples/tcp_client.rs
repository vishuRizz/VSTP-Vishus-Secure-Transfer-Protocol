use std::error::Error;
use std::time::Duration;
use tracing::info;
use vstp::{tcp::VstpTcpClient, types::FrameType};

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    // Initialize tracing
    tracing_subscriber::fmt::init();

    info!("Starting VSTP TCP Client example...");

    // Connect to the server
    let mut client = VstpTcpClient::connect("127.0.0.1:6969").await?;

    // Send HELLO to start the session
    info!("Sending HELLO frame...");
    client.send_hello().await?;

    // Wait a bit for server response
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Send some data
    info!("Sending DATA frame...");
    let message = "Hello from VSTP client!".as_bytes().to_vec();
    client.send_data(message).await?;

    // Wait a bit for server response
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Try to receive any frames from server
    info!("Checking for server responses...");
    for _ in 0..5 {
        match client.recv().await? {
            Some(frame) => {
                info!("Received frame: {:?}", frame.typ);
                match frame.typ {
                    FrameType::Data => {
                        if let Ok(payload_str) = String::from_utf8(frame.payload) {
                            info!("Server data: {}", payload_str);
                        }
                    }
                    _ => {
                        info!("Server sent: {:?}", frame.typ);
                    }
                }
            }
            None => {
                info!("No more frames from server");
                break;
            }
        }
    }

    // Close the connection gracefully
    info!("Closing connection...");
    client.close().await?;

    info!("Client example completed successfully!");
    Ok(())
}
