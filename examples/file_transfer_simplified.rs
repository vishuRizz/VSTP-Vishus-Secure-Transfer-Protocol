use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tokio::fs;
use vstp::easy::{VstpClient, VstpServer};

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
    // Start file server
    let server = VstpServer::bind_tcp("127.0.0.1:8080").await?;
    println!("File server running on 127.0.0.1:8080");

    // Handle file requests
    server.serve(|req: FileRequest| async move {
        let path = PathBuf::from(req.path);
        
        match fs::read(&path).await {
            Ok(content) => Ok(FileResponse {
                name: path.file_name().unwrap().to_string_lossy().into_owned(),
                content,
                error: None,
            }),
            Err(e) => Ok(FileResponse {
                name: path.file_name().unwrap().to_string_lossy().into_owned(),
                content: vec![],
                error: Some(e.to_string()),
            }),
        }
    }).await?;

    Ok(())
}
