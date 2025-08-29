use bytes::{BufMut, BytesMut};
use tokio_util::codec::{Decoder, Encoder};

use crate::frame::{encode_frame, try_decode_frame};
use crate::types::{Frame, VstpError};

/// Tokio codec for VSTP frames
pub struct VstpFrameCodec {
    max_frame_size: usize,
}

impl VstpFrameCodec {
    pub fn new(max_frame_size: usize) -> Self {
        Self { max_frame_size }
    }
    
    pub fn default() -> Self {
        Self::new(8 * 1024 * 1024) // 8MB default
    }
}

impl Decoder for VstpFrameCodec {
    type Item = Frame;
    type Error = VstpError;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        try_decode_frame(src, self.max_frame_size)
    }
}

impl Encoder<Frame> for VstpFrameCodec {
    type Error = VstpError;

    fn encode(&mut self, item: Frame, dst: &mut BytesMut) -> Result<(), Self::Error> {
        let encoded = encode_frame(&item)?;
        dst.put_slice(&encoded);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Frame, FrameType, Flags};

    #[test]
    fn test_codec_roundtrip() {
        let mut codec = VstpFrameCodec::default();
        let mut buf = BytesMut::new();
        
        let frame = Frame::new(FrameType::Data)
            .with_header("test", "value")
            .with_payload(b"hello".to_vec());
        
        // Encode
        codec.encode(frame.clone(), &mut buf).unwrap();
        
        // Decode
        let decoded = codec.decode(&mut buf).unwrap().unwrap();
        
        assert_eq!(frame, decoded);
    }
    
    #[test]
    fn test_codec_partial_decode() {
        let mut codec = VstpFrameCodec::default();
        let mut buf = BytesMut::new();
        
        let frame = Frame::new(FrameType::Hello);
        let encoded = encode_frame(&frame).unwrap();
        
        // Add partial data
        buf.put_slice(&encoded[..5]);
        let result = codec.decode(&mut buf).unwrap();
        assert!(result.is_none());
        
        // Add remaining data
        buf.put_slice(&encoded[5..]);
        let decoded = codec.decode(&mut buf).unwrap().unwrap();
        assert_eq!(frame, decoded);
    }
}
