use byteorder::{BigEndian, LittleEndian, ReadBytesExt};
use bytes::{BufMut, Bytes, BytesMut};
use crc_any::CRC;

use crate::types::{Flags, Frame, FrameType, Header, VstpError, VSTP_MAGIC, VSTP_VERSION};

/// Encode a VSTP frame into bytes according to the wire format specification
pub fn encode_frame(frame: &Frame) -> Result<Bytes, VstpError> {
    let mut buf = BytesMut::new();

    // Fixed header: [MAGIC (2B)] [VER (1B)] [TYPE (1B)] [FLAGS (1B)]
    buf.put_slice(&VSTP_MAGIC);
    buf.put_u8(frame.version);
    buf.put_u8(frame.typ as u8);
    buf.put_u8(frame.flags.bits());

    // Encode headers first to calculate total header length
    let mut header_data = BytesMut::new();
    for header in &frame.headers {
        // Validate header key length
        if header.key.len() > 255 {
            return Err(VstpError::Protocol("Header key too long".to_string()));
        }
        if header.value.len() > 255 {
            return Err(VstpError::Protocol("Header value too long".to_string()));
        }

        // Write header: [KEY_LEN (1B)] [VALUE_LEN (1B)] [KEY] [VALUE]
        header_data.put_u8(header.key.len() as u8);
        header_data.put_u8(header.value.len() as u8);
        header_data.put_slice(&header.key);
        header_data.put_slice(&header.value);
    }

    // Write header length (little-endian) and payload length (big-endian)
    buf.put_u16_le(header_data.len() as u16);
    // Write payload length in big-endian manually
    let payload_len = frame.payload.len() as u32;
    buf.put_u8((payload_len >> 24) as u8);
    buf.put_u8((payload_len >> 16) as u8);
    buf.put_u8((payload_len >> 8) as u8);
    buf.put_u8(payload_len as u8);

    // Write headers and payload
    buf.put_slice(&header_data);
    buf.put_slice(&frame.payload);

    // Calculate CRC over the entire frame (excluding CRC field)
    let mut crc = CRC::crc32();
    crc.digest(&buf);
    let crc_value = crc.get_crc() as u32;

    // Write CRC (big-endian)
    buf.put_u8((crc_value >> 24) as u8);
    buf.put_u8((crc_value >> 16) as u8);
    buf.put_u8((crc_value >> 8) as u8);
    buf.put_u8(crc_value as u8);

    Ok(buf.freeze())
}

/// Try to decode a VSTP frame from a buffer
pub fn try_decode_frame(
    buf: &mut BytesMut,
    max_frame_size: usize,
) -> Result<Option<Frame>, VstpError> {
    // Need at least 11 bytes for fixed header + lengths
    if buf.len() < 11 {
        return Ok(None);
    }

    // Check magic bytes
    if buf[0] != VSTP_MAGIC[0] || buf[1] != VSTP_MAGIC[1] {
        return Err(VstpError::Protocol("Invalid magic bytes".to_string()));
    }

    // Parse fixed header
    let version = buf[2];
    let frame_type = buf[3];
    let flags = buf[4];

    // Validate version
    if version != VSTP_VERSION {
        return Err(VstpError::Protocol("Unsupported version".to_string()));
    }

    // Parse lengths
    let header_len = (&buf[5..7]).read_u16::<LittleEndian>().unwrap() as usize;
    let payload_len = (&buf[7..11]).read_u32::<BigEndian>().unwrap() as usize;

    // Calculate total frame size
    let total_size = 11 + header_len + payload_len + 4; // +4 for CRC

    // Check size limits
    if total_size > max_frame_size {
        return Err(VstpError::Protocol("Frame too large".to_string()));
    }

    // Check if we have enough data
    if buf.len() < total_size {
        return Ok(None);
    }

    // Extract the complete frame
    let frame_data = buf.split_to(total_size);

    // Verify CRC
    let expected_crc = (&frame_data[total_size - 4..])
        .read_u32::<BigEndian>()
        .unwrap();
    let mut crc = CRC::crc32();
    crc.digest(&frame_data[..total_size - 4]);
    let calculated_crc = crc.get_crc() as u32;

    if expected_crc != calculated_crc {
        return Err(VstpError::CrcMismatch {
            expected: expected_crc,
            got: calculated_crc,
        });
    }

    // Parse frame type
    let typ = match frame_type {
        0x01 => FrameType::Hello,
        0x02 => FrameType::Welcome,
        0x03 => FrameType::Data,
        0x04 => FrameType::Ping,
        0x05 => FrameType::Pong,
        0x06 => FrameType::Bye,
        0x07 => FrameType::Ack,
        0x08 => FrameType::Err,
        _ => return Err(VstpError::Protocol("Invalid frame type".to_string())),
    };

    // Parse headers
    let mut headers = Vec::new();
    let mut header_pos = 11; // Start after fixed header

    while header_pos < 11 + header_len {
        if header_pos + 2 > frame_data.len() {
            return Err(VstpError::Protocol("Incomplete header length".to_string()));
        }

        let key_len = frame_data[header_pos] as usize;
        let value_len = frame_data[header_pos + 1] as usize;
        header_pos += 2;

        if header_pos + key_len + value_len > frame_data.len() {
            return Err(VstpError::Protocol("Incomplete header value".to_string()));
        }

        let key = frame_data[header_pos..header_pos + key_len].to_vec();
        header_pos += key_len;
        let value = frame_data[header_pos..header_pos + value_len].to_vec();
        header_pos += value_len;

        headers.push(Header { key, value });
    }

    // Parse payload
    let payload_start = 11 + header_len;
    let payload_end = payload_start + payload_len;
    let payload = frame_data[payload_start..payload_end].to_vec();

    Ok(Some(Frame {
        version,
        typ,
        flags: Flags::from_bits(flags).unwrap_or(Flags::empty()),
        headers,
        payload,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Frame;

    #[test]
    fn test_basic_roundtrip() {
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
            .with_header("msg-id", "12345");

        let encoded = encode_frame(&frame).unwrap();
        let mut buf = BytesMut::from(&encoded[..]);
        let decoded = try_decode_frame(&mut buf, 1024).unwrap().unwrap();

        assert_eq!(frame, decoded);
    }

    #[test]
    fn test_frame_with_payload() {
        let payload = b"This is a test payload with some data".to_vec();
        let frame = Frame::new(FrameType::Data).with_payload(payload);

        let encoded = encode_frame(&frame).unwrap();
        let mut buf = BytesMut::from(&encoded[..]);
        let decoded = try_decode_frame(&mut buf, 1024).unwrap().unwrap();

        assert_eq!(frame, decoded);
    }
}
