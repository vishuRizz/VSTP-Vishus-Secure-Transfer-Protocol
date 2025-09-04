//! VSTP UDP Server Example
//!
//! This example demonstrates how to use the VSTP UDP server to handle incoming frames.

use std::error::Error;
use std::net::SocketAddr;
use tracing::info;
use vstp_labs::{types::FrameType, udp::VstpUdpServer};

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    // Initialize tracing
    tracing_subscriber::fmt::init();

    info!("Starting VSTP UDP Server example...");

    // Bind to UDP port 6969
    let server = VstpUdpServer::bind("127.0.0.1:6969").await?;
    let local_addr = server.local_addr()?;
    info!("Server bound to {}", local_addr);

    // Run the server with a simple frame handler
    server.run(handle_frame).await?;

    Ok(())
}

/// Handle incoming frames
async fn handle_frame(from_addr: SocketAddr, frame: vstp_labs::types::Frame) {
    info!(
        "Received {:?} frame from {} with {} headers and {} bytes payload",
        frame.typ,
        from_addr,
        frame.headers.len(),
        frame.payload.len()
    );

    // Log frame details
    for header in &frame.headers {
        if let (Ok(key), Ok(value)) = (
            std::str::from_utf8(&header.key),
            std::str::from_utf8(&header.value),
        ) {
            info!("  Header: {} = {}", key, value);
        }
    }

    // Handle different frame types
    match frame.typ {
        FrameType::Hello => {
            info!("  -> Client said hello!");
        }
        FrameType::Data => {
            if let Ok(payload_str) = std::str::from_utf8(&frame.payload) {
                info!("  -> Data: {}", payload_str);
            } else {
                info!("  -> Data: {} bytes (binary)", frame.payload.len());
            }
        }
        FrameType::Ping => {
            info!("  -> Ping received");
        }
        FrameType::Bye => {
            info!("  -> Client said goodbye");
        }
        _ => {
            info!("  -> Other frame type: {:?}", frame.typ);
        }
    }
}
