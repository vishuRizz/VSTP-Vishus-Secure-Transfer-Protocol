use serde::{Deserialize, Serialize};
use vstp::easy::{VstpClient, VstpServer};

#[derive(Debug, Serialize, Deserialize)]
struct ChatMessage {
    from: String,
    content: String,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Start chat server
    let server = VstpServer::bind_tcp("127.0.0.1:8080").await?;
    
    println!("Chat server running on 127.0.0.1:8080");
    
    // Handle messages by broadcasting them back to all clients
    server.serve(|msg: ChatMessage| async move {
        println!("{}: {}", msg.from, msg.content);
        Ok(msg) // Echo message back
    }).await?;

    Ok(())
}
