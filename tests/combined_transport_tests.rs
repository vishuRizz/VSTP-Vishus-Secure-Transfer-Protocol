//! Combined tests for TCP and UDP transports

use std::time::Duration;
use tokio::time::timeout;
use vstp::{
    tcp::{VstpTcpClient, VstpTcpServer},
    udp::{VstpUdpClient, VstpUdpServer},
    types::FrameType,
};

#[tokio::test]
async fn test_tcp_and_udp_side_by_side() {
    // Start TCP server
    let tcp_server = VstpTcpServer::bind("127.0.0.1:0").await.unwrap();
    let tcp_server_addr = tcp_server.local_addr().unwrap();
    
    let tcp_server_handle = tokio::spawn(async move {
        tcp_server.run(|_session_id, frame| async move {
            println!("TCP Server received: {:?}", frame.typ);
        }).await.unwrap();
    });

    // Start UDP server
    let udp_server = VstpUdpServer::bind("127.0.0.1:0").await.unwrap();
    let udp_server_addr = udp_server.local_addr().unwrap();
    
    let udp_server_handle = tokio::spawn(async move {
        udp_server.run(|_addr, frame| async move {
            println!("UDP Server received: {:?}", frame.typ);
        }).await.unwrap();
    });

    // Give servers time to start
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Test TCP client
    let mut tcp_client = VstpTcpClient::connect(&format!("127.0.0.1:{}", tcp_server_addr.port())).await.unwrap();
    tcp_client.send_hello().await.unwrap();
    tcp_client.send_data(b"TCP message".to_vec()).await.unwrap();
    tcp_client.close().await.unwrap();

    // Test UDP client
    let udp_client = VstpUdpClient::bind("127.0.0.1:0").await.unwrap();
    let hello_frame = vstp::Frame::new(FrameType::Hello);
    udp_client.send(hello_frame, udp_server_addr).await.unwrap();
    
    let data_frame = vstp::Frame::new(FrameType::Data)
        .with_payload(b"UDP message".to_vec());
    udp_client.send(data_frame, udp_server_addr).await.unwrap();

    // Give time for processing
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Stop servers
    tcp_server_handle.abort();
    udp_server_handle.abort();
}

#[tokio::test]
async fn test_transport_choice_functionality() {
    // This test demonstrates that users can choose between TCP and UDP
    
    // Test TCP transport choice
    let tcp_server = VstpTcpServer::bind("127.0.0.1:0").await.unwrap();
    let tcp_addr = tcp_server.local_addr().unwrap();
    
    let tcp_handle = tokio::spawn(async move {
        tcp_server.run(|_session_id, frame| async move {
            assert_eq!(frame.typ, FrameType::Hello);
            println!("TCP transport working correctly");
        }).await.unwrap();
    });

    tokio::time::sleep(Duration::from_millis(50)).await;

    let mut tcp_client = VstpTcpClient::connect(&format!("127.0.0.1:{}", tcp_addr.port())).await.unwrap();
    tcp_client.send_hello().await.unwrap();
    tcp_client.close().await.unwrap();

    // Test UDP transport choice
    let udp_server = VstpUdpServer::bind("127.0.0.1:0").await.unwrap();
    let udp_addr = udp_server.local_addr().unwrap();
    
    let udp_handle = tokio::spawn(async move {
        udp_server.run(|_addr, frame| async move {
            assert_eq!(frame.typ, FrameType::Hello);
            println!("UDP transport working correctly");
        }).await.unwrap();
    });

    tokio::time::sleep(Duration::from_millis(50)).await;

    let udp_client = VstpUdpClient::bind("127.0.0.1:0").await.unwrap();
    let hello_frame = vstp::Frame::new(FrameType::Hello);
    udp_client.send(hello_frame, udp_addr).await.unwrap();

    tokio::time::sleep(Duration::from_millis(100)).await;

    // Cleanup
    tcp_handle.abort();
    udp_handle.abort();
}

#[tokio::test]
async fn test_udp_reliability_features() {
    // Test UDP-specific features: ACK reliability and fragmentation
    
    let udp_server = VstpUdpServer::bind("127.0.0.1:0").await.unwrap();
    let udp_addr = udp_server.local_addr().unwrap();
    
    let udp_handle = tokio::spawn(async move {
        udp_server.run(|_addr, frame| async move {
            println!("UDP Server received: {:?}", frame.typ);
        }).await.unwrap();
    });

    tokio::time::sleep(Duration::from_millis(50)).await;

    let mut udp_client = VstpUdpClient::bind("127.0.0.1:0").await.unwrap();

    // Test ACK reliability
    let reliable_frame = vstp::Frame::new(FrameType::Data)
        .with_payload(b"Reliable message".to_vec());
    
    let result = timeout(Duration::from_secs(3), udp_client.send_with_ack(reliable_frame, udp_addr)).await;
    assert!(result.is_ok(), "ACK reliability should work");

    // Test fragmentation with large payload
    let large_payload = vec![0x42u8; 2000];
    let large_frame = vstp::Frame::new(FrameType::Data)
        .with_payload(large_payload);
    
    udp_client.send(large_frame, udp_addr).await.unwrap();

    tokio::time::sleep(Duration::from_millis(200)).await;

    udp_handle.abort();
}
