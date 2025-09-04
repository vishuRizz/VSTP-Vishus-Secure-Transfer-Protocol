//! Integration tests for VSTP UDP functionality

use std::time::Duration;
use tokio::time::timeout;
use vstp::{udp::{VstpUdpClient, VstpUdpServer}, types::FrameType};

#[tokio::test]
async fn test_udp_client_server_communication() {
    // Start a UDP server
    let server = VstpUdpServer::bind("127.0.0.1:0").await.unwrap();
    let server_addr = server.local_addr().unwrap();
    
    // Spawn server task
    let server_handle = tokio::spawn(async move {
        server.run(|_addr, frame| async move {
            println!("Server received frame: {:?}", frame.typ);
        }).await.unwrap();
    });

    // Give server time to start
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Create UDP client
    let client = VstpUdpClient::bind("127.0.0.1:0").await.unwrap();

    // Send HELLO frame
    let hello_frame = vstp::Frame::new(FrameType::Hello);
    client.send(hello_frame, server_addr).await.unwrap();

    // Send DATA frame
    let data_frame = vstp::Frame::new(FrameType::Data)
        .with_payload(b"Hello UDP!".to_vec());
    client.send(data_frame, server_addr).await.unwrap();

    // Send BYE frame
    let bye_frame = vstp::Frame::new(FrameType::Bye);
    client.send(bye_frame, server_addr).await.unwrap();

    // Give server time to process
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Stop the server
    server_handle.abort();
}

#[tokio::test]
async fn test_udp_ack_reliability() {
    // Start a UDP server
    let server = VstpUdpServer::bind("127.0.0.1:0").await.unwrap();
    let server_addr = server.local_addr().unwrap();
    
    // Spawn server task
    let server_handle = tokio::spawn(async move {
        server.run(|_addr, frame| async move {
            println!("Server received frame: {:?}", frame.typ);
        }).await.unwrap();
    });

    // Give server time to start
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Create UDP client
    let mut client = VstpUdpClient::bind("127.0.0.1:0").await.unwrap();

    // Send DATA frame with ACK reliability
    let data_frame = vstp::Frame::new(FrameType::Data)
        .with_payload(b"Reliable message!".to_vec());
    
    // This should succeed with ACK
    let result = timeout(Duration::from_secs(5), client.send_with_ack(data_frame, server_addr)).await;
    assert!(result.is_ok(), "send_with_ack should succeed");
    assert!(result.unwrap().is_ok(), "send_with_ack should return Ok");

    // Stop the server
    server_handle.abort();
}

#[tokio::test]
async fn test_udp_fragmentation() {
    // Start a UDP server
    let server = VstpUdpServer::bind("127.0.0.1:0").await.unwrap();
    let server_addr = server.local_addr().unwrap();
    
    // Spawn server task
    let server_handle = tokio::spawn(async move {
        server.run(|_addr, frame| async move {
            println!("Server received frame: {:?} with {} bytes", frame.typ, frame.payload.len());
        }).await.unwrap();
    });

    // Give server time to start
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Create UDP client
    let client = VstpUdpClient::bind("127.0.0.1:0").await.unwrap();

    // Create a large payload that will require fragmentation
    let large_payload = vec![0x42u8; 2000]; // Larger than MAX_DATAGRAM_SIZE
    let data_frame = vstp::Frame::new(FrameType::Data)
        .with_payload(large_payload);

    // Send the large frame (should be fragmented)
    client.send(data_frame, server_addr).await.unwrap();

    // Give server time to process and reassemble
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Stop the server
    server_handle.abort();
}

#[tokio::test]
async fn test_udp_multiple_clients() {
    // Start a UDP server
    let server = VstpUdpServer::bind("127.0.0.1:0").await.unwrap();
    let server_addr = server.local_addr().unwrap();
    
    // Spawn server task
    let server_handle = tokio::spawn(async move {
        server.run(|addr, frame| async move {
            println!("Server received frame from {}: {:?}", addr, frame.typ);
        }).await.unwrap();
    });

    // Give server time to start
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Create multiple clients
    let mut clients = Vec::new();
    for i in 0..3 {
        let client = VstpUdpClient::bind("127.0.0.1:0").await.unwrap();
        let message = format!("Hello from client {}!", i);
        let data_frame = vstp::Frame::new(FrameType::Data)
            .with_payload(message.as_bytes().to_vec());
        client.send(data_frame, server_addr).await.unwrap();
        clients.push(client);
    }

    // Give server time to process
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Stop the server
    server_handle.abort();
}
