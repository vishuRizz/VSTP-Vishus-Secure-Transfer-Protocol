//! Advanced VSTP Features Demonstration
//!
//! This example showcases all the powerful features of VSTP:
//! - Complex headers and metadata
//! - Large payload fragmentation
//! - CRC integrity checking
//! - ACK reliability
//! - Mixed transport usage

use std::error::Error;
use std::time::Duration;
use tokio::time::timeout;
use tracing::info;
use vstp::{
    tcp::{VstpTcpClient, VstpTcpServer},
    udp::{VstpUdpClient, VstpUdpServer},
    types::{Frame, FrameType, Flags},
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    // Initialize tracing
    tracing_subscriber::fmt::init();

    info!("🚀 Starting VSTP Advanced Features Demonstration...");

    // Start both TCP and UDP servers
    let tcp_server = VstpTcpServer::bind("127.0.0.1:6969").await?;
    let udp_server = VstpUdpServer::bind("127.0.0.1:6970").await?;

    // Spawn servers
    let tcp_handle = tokio::spawn(async move {
        tcp_server.run(|session_id, frame| async move {
            info!("🔗 TCP Session {} received:", session_id);
            info!("   📋 Headers: {}", frame.headers.len());
            info!("   📦 Payload: {} bytes", frame.payload.len());
            info!("   🏷️  Flags: {:?}", frame.flags);
            
            // Show some headers
            for header in &frame.headers[..3.min(frame.headers.len())] {
                if let (Ok(key), Ok(value)) = (
                    std::str::from_utf8(&header.key),
                    std::str::from_utf8(&header.value)
                ) {
                    info!("   📝 {}: {}", key, value);
                }
            }
        }).await.unwrap();
    });

    let udp_handle = tokio::spawn(async move {
        udp_server.run(|addr, frame| async move {
            info!("📡 UDP from {} received:", addr);
            info!("   📋 Headers: {}", frame.headers.len());
            info!("   📦 Payload: {} bytes", frame.payload.len());
            info!("   🏷️  Flags: {:?}", frame.flags);
            
            // Show some headers
            for header in &frame.headers[..3.min(frame.headers.len())] {
                if let (Ok(key), Ok(value)) = (
                    std::str::from_utf8(&header.key),
                    std::str::from_utf8(&header.value)
                ) {
                    info!("   📝 {}: {}", key, value);
                }
            }
        }).await.unwrap();
    });

    tokio::time::sleep(Duration::from_millis(100)).await;

    // Demo 1: TCP with complex metadata
    info!("🎯 Demo 1: TCP with Complex Metadata");
    let mut tcp_client = VstpTcpClient::connect("127.0.0.1:6969").await?;
    
    let complex_tcp_frame = Frame::new(FrameType::Data)
        .with_header("content-type", "application/json")
        .with_header("user-id", "12345")
        .with_header("session-id", "sess-abc123")
        .with_header("api-version", "2.1")
        .with_header("compression", "gzip")
        .with_header("cache-control", "no-cache")
        .with_header("request-id", "req-789")
        .with_header("priority", "high")
        .with_header("timeout", "30")
        .with_header("retry-count", "3")
        .with_header("auth-token", "bearer_xyz123")
        .with_header("user-agent", "VSTP-Demo/1.0")
        .with_payload(br#"{
            "message": "Hello from VSTP TCP!",
            "data": {
                "users": ["alice", "bob", "charlie"],
                "settings": {"theme": "dark", "notifications": true},
                "metadata": {"timestamp": 1640995200, "version": "1.0"}
            }
        }"#.to_vec())
        .with_flag(Flags::CRC);

    tcp_client.send(complex_tcp_frame).await?;
    tcp_client.close().await?;

    tokio::time::sleep(Duration::from_millis(200)).await;

    // Demo 2: UDP with massive payload (will fragment)
    info!("🎯 Demo 2: UDP with Massive Payload (Auto-Fragmentation)");
    let mut udp_client = VstpUdpClient::bind("127.0.0.1:0").await?;
    
    // Create a massive payload that will definitely fragment
    let massive_payload = vec![0x42u8; 50000]; // 50KB
    let massive_frame = Frame::new(FrameType::Data)
        .with_header("file-name", "massive-dataset.bin")
        .with_header("file-size", "50000")
        .with_header("file-type", "binary")
        .with_header("compression", "none")
        .with_header("checksum", "sha256:abc123def456")
        .with_header("upload-id", "upload-789")
        .with_header("chunk-id", "001")
        .with_header("total-chunks", "1")
        .with_payload(massive_payload)
        .with_flag(Flags::REQ_ACK);

    info!("📤 Sending 50KB payload (will auto-fragment)...");
    let result = timeout(Duration::from_secs(10), 
                        udp_client.send_with_ack(massive_frame, "127.0.0.1:6970".parse()?)).await;
    
    if result.is_ok() && result.unwrap().is_ok() {
        info!("✅ Massive payload sent and ACK received!");
    } else {
        info!("❌ Massive payload transfer failed or timed out");
    }

    tokio::time::sleep(Duration::from_millis(500)).await;

    // Demo 3: Multiple data types
    info!("🎯 Demo 3: Multiple Data Types");
    
    // JSON data
    let json_frame = Frame::new(FrameType::Data)
        .with_header("data-type", "json")
        .with_header("encoding", "utf-8")
        .with_payload(br#"{"users": [{"id": 1, "name": "Alice"}, {"id": 2, "name": "Bob"}]}"#.to_vec());
    
    udp_client.send(json_frame, "127.0.0.1:6970".parse()?).await?;

    // Binary data
    let binary_frame = Frame::new(FrameType::Data)
        .with_header("data-type", "binary")
        .with_header("format", "raw")
        .with_payload(vec![0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A]);
    
    udp_client.send(binary_frame, "127.0.0.1:6970".parse()?).await?;

    // Text data
    let text_frame = Frame::new(FrameType::Data)
        .with_header("data-type", "text")
        .with_header("language", "en")
        .with_payload("Hello, VSTP World! This is a text message with special chars: !@#$%^&*()".as_bytes().to_vec());
    
    udp_client.send(text_frame, "127.0.0.1:6970".parse()?).await?;

    tokio::time::sleep(Duration::from_millis(300)).await;

    // Demo 4: Gaming scenario
    info!("🎯 Demo 4: Gaming Scenario (Low Latency)");
    
    let game_state = Frame::new(FrameType::Data)
        .with_header("game-id", "match-456")
        .with_header("player-id", "player-789")
        .with_header("tick", "1234")
        .with_header("latency", "ultra-low")
        .with_header("priority", "critical")
        .with_payload(br#"{
            "players": [
                {"id": 1, "x": 100, "y": 200, "health": 85},
                {"id": 2, "x": 150, "y": 250, "health": 92}
            ],
            "world": {"time": 1234, "weather": "sunny"}
        }"#.to_vec());
    
    udp_client.send(game_state, "127.0.0.1:6970".parse()?).await?;

    // Demo 5: IoT sensor data
    info!("🎯 Demo 5: IoT Sensor Data");
    
    let sensor_data = Frame::new(FrameType::Data)
        .with_header("sensor-id", "temp-001")
        .with_header("location", "building-a-floor-2")
        .with_header("battery", "85%")
        .with_header("timestamp", "1640995200")
        .with_header("accuracy", "high")
        .with_header("calibration", "auto")
        .with_payload(br#"{
            "temperature": 23.5,
            "humidity": 45.2,
            "pressure": 1013.25,
            "light": 850,
            "motion": false
        }"#.to_vec())
        .with_flag(Flags::REQ_ACK);
    
    let result = timeout(Duration::from_secs(5), 
                        udp_client.send_with_ack(sensor_data, "127.0.0.1:6970".parse()?)).await;
    
    if result.is_ok() && result.unwrap().is_ok() {
        info!("✅ IoT sensor data sent reliably!");
    }

    tokio::time::sleep(Duration::from_millis(200)).await;

    // Demo 6: File transfer simulation
    info!("🎯 Demo 6: File Transfer Simulation");
    
    for chunk_id in 1..=5 {
        let chunk_data = vec![chunk_id as u8; 1000]; // 1KB chunks
        let file_chunk = Frame::new(FrameType::Data)
            .with_header("file-id", "doc-123")
            .with_header("chunk-number", &chunk_id.to_string())
            .with_header("total-chunks", "5")
            .with_header("file-size", "5000")
            .with_header("file-name", "document.pdf")
            .with_header("checksum", "md5:abc123")
            .with_payload(chunk_data)
            .with_flag(Flags::REQ_ACK);
        
        let result = timeout(Duration::from_secs(3), 
                            udp_client.send_with_ack(file_chunk, "127.0.0.1:6970".parse()?)).await;
        
        if result.is_ok() && result.unwrap().is_ok() {
            info!("✅ File chunk {}/5 sent successfully", chunk_id);
        } else {
            info!("❌ File chunk {}/5 failed", chunk_id);
        }
        
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    tokio::time::sleep(Duration::from_millis(500)).await;

    info!("🎉 Advanced Features Demonstration Complete!");
    info!("📊 Summary:");
    info!("   ✅ Complex metadata handling");
    info!("   ✅ Massive payload fragmentation");
    info!("   ✅ CRC integrity checking");
    info!("   ✅ ACK reliability mechanism");
    info!("   ✅ Multiple data types");
    info!("   ✅ Gaming scenarios");
    info!("   ✅ IoT sensor data");
    info!("   ✅ File transfer simulation");

    // Cleanup
    tcp_handle.abort();
    udp_handle.abort();

    Ok(())
}
