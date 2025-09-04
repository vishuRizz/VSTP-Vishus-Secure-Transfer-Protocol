use bytes::{BufMut, BytesMut};
use vstp::{encode_frame, try_decode_frame, Flags, Frame, FrameType, Header};

#[test]
fn test_basic_frame_roundtrip() {
    let frame = Frame::new(FrameType::Hello);
    let encoded = encode_frame(&frame).unwrap();
    let mut buf = BytesMut::from(&encoded[..]);
    let decoded = try_decode_frame(&mut buf, 1024).unwrap().unwrap();

    assert_eq!(frame, decoded);
}

#[test]
fn test_frame_with_headers() {
    let frame = Frame::new(FrameType::Data)
        .with_header("content-type", "application/json")
        .with_header("msg-id", "12345")
        .with_header("session-id", "abc-def-ghi");

    let encoded = encode_frame(&frame).unwrap();
    let mut buf = BytesMut::from(&encoded[..]);
    let decoded = try_decode_frame(&mut buf, 1024).unwrap().unwrap();

    assert_eq!(frame, decoded);
    assert_eq!(decoded.headers.len(), 3);
}

#[test]
fn test_frame_with_payload() {
    let payload = b"This is a test payload with some data".to_vec();
    let frame = Frame::new(FrameType::Data).with_payload(payload.clone());

    let encoded = encode_frame(&frame).unwrap();
    let mut buf = BytesMut::from(&encoded[..]);
    let decoded = try_decode_frame(&mut buf, 1024).unwrap().unwrap();

    assert_eq!(frame, decoded);
    assert_eq!(decoded.payload, payload);
}

#[test]
fn test_frame_with_flags() {
    let frame = Frame::new(FrameType::Data)
        .with_flag(Flags::REQ_ACK | Flags::CRC)
        .with_payload(b"test".to_vec());

    let encoded = encode_frame(&frame).unwrap();
    let mut buf = BytesMut::from(&encoded[..]);
    let decoded = try_decode_frame(&mut buf, 1024).unwrap().unwrap();

    assert_eq!(frame, decoded);
    assert!(decoded.flags.contains(Flags::REQ_ACK));
    assert!(decoded.flags.contains(Flags::CRC));
}

#[test]
fn test_all_frame_types() {
    let frame_types = [
        FrameType::Hello,
        FrameType::Welcome,
        FrameType::Data,
        FrameType::Ping,
        FrameType::Pong,
        FrameType::Bye,
        FrameType::Ack,
        FrameType::Err,
    ];

    for frame_type in frame_types {
        let frame = Frame::new(frame_type);
        let encoded = encode_frame(&frame).unwrap();
        let mut buf = BytesMut::from(&encoded[..]);
        let decoded = try_decode_frame(&mut buf, 1024).unwrap().unwrap();

        assert_eq!(frame, decoded);
        assert_eq!(decoded.typ, frame_type);
    }
}

#[test]
fn test_large_payload() {
    let payload = vec![0x42; 10000]; // 10KB payload
    let frame = Frame::new(FrameType::Data).with_payload(payload.clone());

    let encoded = encode_frame(&frame).unwrap();
    let mut buf = BytesMut::from(&encoded[..]);
    let decoded = try_decode_frame(&mut buf, 1024 * 1024).unwrap().unwrap();

    assert_eq!(frame, decoded);
    assert_eq!(decoded.payload.len(), 10000);
}

#[test]
fn test_crc_validation() {
    let frame = Frame::new(FrameType::Data)
        .with_flag(Flags::CRC)
        .with_header("test", "value")
        .with_payload(b"payload".to_vec());

    let encoded = encode_frame(&frame).unwrap();
    let mut buf = BytesMut::from(&encoded[..]);
    let decoded = try_decode_frame(&mut buf, 1024).unwrap().unwrap();

    assert_eq!(frame, decoded);

    // Test CRC corruption
    let mut corrupted = encoded.to_vec();
    let len = corrupted.len();
    if len >= 2 {
        corrupted[len - 2] ^= 0xFF; // Flip CRC bits
    }
    let mut buf = BytesMut::from(&corrupted[..]);
    let result = try_decode_frame(&mut buf, 1024);
    assert!(result.is_err());
}

#[test]
fn test_incomplete_frame() {
    let frame = Frame::new(FrameType::Hello);
    let encoded = encode_frame(&frame).unwrap();

    // Test with various partial lengths
    for i in 1..encoded.len() {
        let mut buf = BytesMut::from(&encoded[..i]);
        let result = try_decode_frame(&mut buf, 1024).unwrap();
        assert!(
            result.is_none(),
            "Should return None for incomplete frame of length {}",
            i
        );
    }
}

#[test]
fn test_frame_size_limit() {
    let payload = vec![0x42; 1000];
    let frame = Frame::new(FrameType::Data).with_payload(payload);

    let encoded = encode_frame(&frame).unwrap();
    let mut buf = BytesMut::from(&encoded[..]);

    // Should succeed with large enough limit
    let result = try_decode_frame(&mut buf, 2000).unwrap();
    assert!(result.is_some());

    // Should fail with small limit
    let mut buf = BytesMut::from(&encoded[..]);
    let result = try_decode_frame(&mut buf, 100);
    assert!(result.is_err());
}

#[test]
fn test_header_validation() {
    // Test header key too long
    let mut frame = Frame::new(FrameType::Data);
    frame
        .headers
        .push(Header::new(vec![0x41; 256], b"value".to_vec()));
    let result = encode_frame(&frame);
    assert!(result.is_err());

    // Test header value too long
    let mut frame = Frame::new(FrameType::Data);
    frame
        .headers
        .push(Header::new(b"key".to_vec(), vec![0x42; 65536]));
    let result = encode_frame(&frame);
    assert!(result.is_err());
}

#[test]
fn test_malformed_frame() {
    // Test invalid magic
    let mut buf = BytesMut::new();
    buf.put_slice(b"XX"); // Wrong magic
    buf.put_u8(0x01); // version
    buf.put_u8(0x01); // type
    buf.put_u8(0x00); // flags
    buf.put_u16_le(0); // header len
                       // Write payload length in big-endian manually
    let payload_len = 0u32;
    buf.put_u8((payload_len >> 24) as u8);
    buf.put_u8((payload_len >> 16) as u8);
    buf.put_u8((payload_len >> 8) as u8);
    buf.put_u8(payload_len as u8);
    // Add dummy CRC
    buf.put_u32(0x12345678);

    let result = try_decode_frame(&mut buf, 1024);
    assert!(result.is_err());

    // Test invalid version
    let mut buf = BytesMut::new();
    buf.put_slice(b"VT"); // Correct magic
    buf.put_u8(0x99); // Wrong version
    buf.put_u8(0x01); // type
    buf.put_u8(0x00); // flags
    buf.put_u16_le(0); // header len
                       // Write payload length in big-endian manually
    let payload_len = 0u32;
    buf.put_u8((payload_len >> 24) as u8);
    buf.put_u8((payload_len >> 16) as u8);
    buf.put_u8((payload_len >> 8) as u8);
    buf.put_u8(payload_len as u8);
    // Add dummy CRC
    buf.put_u32(0x12345678);

    let result = try_decode_frame(&mut buf, 1024);
    assert!(result.is_err());

    // Test invalid frame type
    let mut buf = BytesMut::new();
    buf.put_slice(b"VT"); // Correct magic
    buf.put_u8(0x01); // version
    buf.put_u8(0x99); // Invalid type
    buf.put_u8(0x00); // flags
    buf.put_u16_le(0); // header len
                       // Write payload length in big-endian manually
    let payload_len = 0u32;
    buf.put_u8((payload_len >> 24) as u8);
    buf.put_u8((payload_len >> 16) as u8);
    buf.put_u8((payload_len >> 8) as u8);
    buf.put_u8(payload_len as u8);
    // Add dummy CRC
    buf.put_u32(0x12345678);

    let result = try_decode_frame(&mut buf, 1024);
    assert!(result.is_err());
}

#[test]
fn test_complex_frame() {
    let frame = Frame::new(FrameType::Data)
        .with_header("content-type", "application/json")
        .with_header("encoding", "utf8")
        .with_header("msg-id", "msg-123")
        .with_header("corr-id", "corr-456")
        .with_flag(Flags::REQ_ACK | Flags::CRC)
        .with_payload(br#"{"message": "Hello, VSTP!", "timestamp": 1234567890}"#.to_vec());

    let encoded = encode_frame(&frame).unwrap();
    let mut buf = BytesMut::from(&encoded[..]);
    let decoded = try_decode_frame(&mut buf, 1024).unwrap().unwrap();

    assert_eq!(frame, decoded);

    // Verify headers
    let headers: std::collections::HashMap<_, _> = decoded
        .headers
        .iter()
        .map(|h| {
            (
                String::from_utf8_lossy(&h.key).to_string(),
                String::from_utf8_lossy(&h.value).to_string(),
            )
        })
        .collect();

    assert_eq!(
        headers.get("content-type"),
        Some(&"application/json".to_string())
    );
    assert_eq!(headers.get("encoding"), Some(&"utf8".to_string()));
    assert_eq!(headers.get("msg-id"), Some(&"msg-123".to_string()));
    assert_eq!(headers.get("corr-id"), Some(&"corr-456".to_string()));

    // Verify payload
    let payload_str = String::from_utf8_lossy(&decoded.payload);
    assert!(payload_str.contains("Hello, VSTP!"));
    assert!(payload_str.contains("1234567890"));
}
