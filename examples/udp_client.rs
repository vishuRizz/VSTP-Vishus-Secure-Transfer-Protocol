//! VSTP UDP Client Example
//!
//! This example demonstrates how to use the VSTP UDP client to send frames to a server.

use std::error::Error;
use std::time::Duration;
use tracing::info;
use vstp::{types::FrameType, udp::VstpUdpClient};

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    // Initialize tracing
    tracing_subscriber::fmt::init();

    info!("Starting VSTP UDP Client example...");

    // Bind to a local address (let the system choose a port)
    let mut client = VstpUdpClient::bind("127.0.0.1:0").await?;
    let local_addr = client.local_addr()?;
    info!("Client bound to {}", local_addr);

    // Connect to the server
    let server_addr = "127.0.0.1:6969".parse()?;
    info!("Connecting to server at {}", server_addr);

    // Send HELLO frame
    info!("Sending HELLO frame...");
    let hello_frame = vstp::Frame::new(FrameType::Hello);
    client.send(hello_frame, server_addr).await?;

    // Wait a bit
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Send DATA frame
    info!("Sending DATA frame...");
    let message = "Hello from VSTP UDP client!";
    let data_frame =
        vstp::Frame::new(FrameType::Data).with_payload(message.as_bytes().to_vec());
    client.send(data_frame, server_addr).await?;

    // Wait a bit
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Send DATA frame with ACK reliability
    info!("Sending DATA frame with ACK reliability...");
    let reliable_message = "This message requires ACK!";
    let reliable_frame = vstp::Frame::new(FrameType::Data)
        .with_payload(reliable_message.as_bytes().to_vec());
    client.send_with_ack(reliable_frame, server_addr).await?;

    // Wait a bit
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Send PING frame
    info!("Sending PING frame...");
    let ping_frame = vstp::Frame::new(FrameType::Ping);
    client.send(ping_frame, server_addr).await?;

    // Wait a bit
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Send BYE frame
    info!("Sending BYE frame...");
    let bye_frame = vstp::Frame::new(FrameType::Bye);
    client.send(bye_frame, server_addr).await?;

    info!("Client example completed successfully!");
    Ok(())
}
