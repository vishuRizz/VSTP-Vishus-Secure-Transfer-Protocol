//! # VSTP - Vishu's Secure Transfer Protocol
//! 
//! A general-purpose, binary, extensible application-layer protocol designed to be:
//! 
//! * **Secure by default** on TCP (TLS 1.3)
//! * **Fast** on UDP (no TLS initially)
//! * **Minimal but extensible** with binary headers
//! * **Easy to implement** across languages
//! 
//! ## Quick Start
//! 
//! ```rust
//! use vstp_labs::{Frame, FrameType, Flags};
//! 
//! // Create a simple data frame
//! let frame = Frame::new(FrameType::Data)
//!     .with_header("content-type", "text/plain")
//!     .with_payload(b"Hello, VSTP!".to_vec())
//!     .with_flag(Flags::REQ_ACK);
//! 
//! // Encode to bytes
//! let encoded = vstp_labs::frame::encode_frame(&frame)?;
//! 
//! // Decode from bytes
//! let mut buf = bytes::BytesMut::from(&encoded[..]);
//! let decoded = vstp_labs::frame::try_decode_frame(&mut buf, 1024)?.unwrap();
//! 
//! assert_eq!(frame, decoded);
//! # Ok::<(), vstp_labs::VstpError>(())
//! ```
//! 
//! ## Protocol Overview
//! 
//! VSTP frames have the following wire format:
//! 
//! 
//! VSTP frames have the following wire format:
//! 
//! - MAGIC (2B): Protocol identifier
//! - VER (1B): Protocol version  
//! - TYPE (1B): Message type
//! - FLAGS (1B): Bit flags
//! - HDR_LEN (2B LE): Header section length (little-endian)
//! - PAY_LEN (4B BE): Payload length (big-endian)
//! - HEADERS: Variable-length header section
//! - PAYLOAD: Variable-length payload section
//! - CRC32: 32-bit checksum
//! 
//! Where:
//! - **MAGIC**: `0x56 0x54` ("VT") to identify VSTP
//! - **VER**: Protocol version (`0x01` for v1)
//! - **TYPE**: Message type (Hello, Welcome, Data, etc.)
//! - **FLAGS**: Bit flags (REQ_ACK, CRC, FRAG, COMP)
//! - **HDR_LEN**: Little-endian header section length
//! - **PAY_LEN**: Big-endian payload length
//! - **HEADERS**: Concatenated binary K/V entries
//! - **PAYLOAD**: Raw bytes (UTF-8 text, JSON, binary, etc.)
//! - **CHECKSUM**: CRC16-IBM over HEADERS|PAYLOAD (optional)
//! 
//! ## Transport Modes
//! 
//! - **TCP mode**: Reliable + encrypted (TLS 1.3 via rustls)
//! - **UDP mode**: Connectionless + fast (no TLS in v0.1)
//! 
//! ## Message Types
//! 
//! | Type | Name    | Direction       | Description                    |
//! |------|---------|-----------------|--------------------------------|
//! | 0x01 | HELLO   | Client → Server | Start of session              |
//! | 0x02 | WELCOME | Server → Client | Server accept                 |
//! | 0x03 | DATA    | Both            | Application data              |
//! | 0x04 | PING    | Both            | Keepalive request             |
//! | 0x05 | PONG    | Both            | Keepalive response            |
//! | 0x06 | BYE     | Both            | Graceful close                |
//! | 0x07 | ACK     | Both            | Acknowledgement               |
//! | 0x08 | ERR     | Both            | Error frame                   |

pub mod types;
pub mod frame;
pub mod codec;
pub mod tcp;

// Re-export main types for convenience
pub use types::{
    Frame, FrameType, Header, Flags, VstpError, SessionId,
    VSTP_MAGIC, VSTP_VERSION,
};

pub use frame::{encode_frame, try_decode_frame};
pub use codec::VstpFrameCodec;
