use serde::{Deserialize, Serialize};
use tokio::io::{self, AsyncBufReadExt};
use vstp::easy::VstpClient;

#[derive(Debug, Serialize, Deserialize)]
struct ChatMessage {
    from: String,
    content: String,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("Enter your name: ");
    let mut name = String::new();
    io::stdin().read_line(&mut name).await?;
    let name = name.trim().to_string();

    // Connect to chat server
    let mut client = VstpClient::connect_tcp("127.0.0.1:8080").await?;
    println!("Connected to chat server!");

    // Split into send and receive tasks
    let mut stdin = io::BufReader::new(io::stdin()).lines();
    
    // Handle incoming messages
    let mut receive_client = client.clone();
    tokio::spawn(async move {
        while let Ok(msg) = receive_client.receive::<ChatMessage>().await {
            if msg.from != name {
                println!("{}: {}", msg.from, msg.content);
            }
        }
    });

    // Handle user input
    println!("Start typing messages (press Enter to send):");
    while let Some(Ok(line)) = stdin.next_line().await {
        let msg = ChatMessage {
            from: name.clone(),
            content: line,
        };
        client.send(msg).await?;
    }

    Ok(())
}
