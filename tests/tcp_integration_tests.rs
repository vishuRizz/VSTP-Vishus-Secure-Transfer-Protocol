use std::time::Duration;
use tokio::time::timeout;
use vstp::{
    tcp::{VstpTcpClient, VstpTcpServer},
    types::{Frame, FrameType, SessionId},
};

#[tokio::test]
async fn test_tcp_client_server_communication() {
    // Start server in background
    let server = VstpTcpServer::bind("127.0.0.1:0").await.unwrap();
    let server_addr = server.local_addr().unwrap();

    let server_handle = tokio::spawn(async move {
        server
            .run(|session_id: SessionId, frame: Frame| async move {
                // Simple echo handler
                match frame.typ {
                    FrameType::Hello => {
                        // Send welcome back
                        println!("Server: Received HELLO from session {}", session_id);
                    }
                    FrameType::Data => {
                        println!(
                            "Server: Received DATA from session {}: {} bytes",
                            session_id,
                            frame.payload.len()
                        );
                    }
                    FrameType::Bye => {
                        println!("Server: Received BYE from session {}", session_id);
                    }
                    _ => {
                        println!(
                            "Server: Received {:?} from session {}",
                            frame.typ, session_id
                        );
                    }
                }
            })
            .await
            .unwrap();
    });

    // Give server time to start
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Connect client
    let mut client = VstpTcpClient::connect(&format!("127.0.0.1:{}", server_addr.port()))
        .await
        .unwrap();

    // Send HELLO
    client.send_hello().await.unwrap();

    // Send DATA
    let test_payload = b"Hello, VSTP!".to_vec();
    client.send_data(test_payload.clone()).await.unwrap();

    // Wait a bit for server processing
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Close connection
    client.close().await.unwrap();

    // Wait for server to finish
    let _ = timeout(Duration::from_secs(5), server_handle).await;

    println!("TCP integration test completed successfully!");
}

#[tokio::test]
async fn test_tcp_multiple_clients() {
    // Start server
    let server = VstpTcpServer::bind("127.0.0.1:0").await.unwrap();
    let server_addr = server.local_addr().unwrap();

    let server_handle = tokio::spawn(async move {
        server
            .run(|session_id: SessionId, frame: Frame| async move {
                println!("Server: Session {} received {:?}", session_id, frame.typ);
            })
            .await
            .unwrap();
    });

    // Give server time to start
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Connect multiple clients
    let mut clients = Vec::new();
    for i in 0..3 {
        let client = VstpTcpClient::connect(&format!("127.0.0.1:{}", server_addr.port()))
            .await
            .unwrap();
        clients.push(client);
        println!("Client {} connected", i);
    }

    // Send data from each client
    for (i, client) in clients.iter_mut().enumerate() {
        let payload = format!("Hello from client {}!", i).as_bytes().to_vec();
        client.send_data(payload).await.unwrap();
        println!("Client {} sent data", i);
    }

    // Wait for processing
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Close all clients
    for (i, client) in clients.iter_mut().enumerate() {
        client.close().await.unwrap();
        println!("Client {} closed", i);
    }

    // Wait for server to finish
    let _ = timeout(Duration::from_secs(5), server_handle).await;

    println!("Multiple clients test completed successfully!");
}
