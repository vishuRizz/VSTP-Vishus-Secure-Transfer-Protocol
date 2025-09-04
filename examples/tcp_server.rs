use std::error::Error;
use tracing::info;
use vstp::{
    tcp::VstpTcpServer,
    types::{Frame, FrameType, SessionId},
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    // Initialize tracing
    tracing_subscriber::fmt::init();

    info!("Starting VSTP TCP Server example...");

    // Bind to default port
    let server = VstpTcpServer::bind("127.0.0.1:6969").await?;

    // Run the server with our handler
    server.run(handle_frame).await?;

    Ok(())
}

/// Handle incoming frames from clients
async fn handle_frame(session_id: SessionId, frame: Frame) {
    info!("Session {}: Received {:?} frame", session_id, frame.typ);

    match frame.typ {
        FrameType::Hello => {
            info!("Session {}: Client said HELLO, sending WELCOME", session_id);
            // In a real implementation, you'd send a WELCOME frame back
        }
        FrameType::Data => {
            if let Ok(payload_str) = String::from_utf8(frame.payload.clone()) {
                info!("Session {}: Received data: {}", session_id, payload_str);
            } else {
                info!(
                    "Session {}: Received binary data ({} bytes)",
                    session_id,
                    frame.payload.len()
                );
            }
        }
        FrameType::Bye => {
            info!("Session {}: Client said BYE", session_id);
        }
        FrameType::Ping => {
            info!("Session {}: Received PING", session_id);
            // In a real implementation, you'd send PONG back
        }
        _ => {
            info!(
                "Session {}: Received frame type: {:?}",
                session_id, frame.typ
            );
        }
    }
}
