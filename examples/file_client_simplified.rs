use serde::{Deserialize, Serialize};
use tokio::fs;
use vstp::easy::VstpClient;

#[derive(Debug, Serialize, Deserialize)]
struct FileRequest {
    path: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct FileResponse {
    name: String,
    content: Vec<u8>,
    error: Option<String>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 2 {
        println!("Usage: {} <file_path>", args[0]);
        return Ok(());
    }

    // Connect to file server
    let mut client = VstpClient::connect_tcp("127.0.0.1:8080").await?;
    println!("Connected to file server!");

    // Request file
    let request = FileRequest {
        path: args[1].clone(),
    };
    
    client.send(request).await?;
    
    // Receive file
    let response: FileResponse = client.receive().await?;
    
    match response.error {
        Some(error) => println!("Error: {}", error),
        None => {
            // Save file
            let output_path = format!("downloaded_{}", response.name);
            fs::write(&output_path, response.content).await?;
            println!("File saved as: {}", output_path);
        }
    }

    Ok(())
}
