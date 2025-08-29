# VSTP Step 1 Documentation: Frame Encoding/Decoding

## Overview

Step 1 implements the core VSTP wire format specification, providing frame encoding and decoding functionality. This is the foundation upon which all other VSTP features are built.

## Wire Format Specification

### Frame Structure

```
[MAGIC (2B)] [VER (1B)] [TYPE (1B)] [FLAGS (1B)] [HEADER_LEN (2B)] [PAYLOAD_LEN (4B)] [HEADERS] [PAYLOAD] [CRC (4B)]
```

- **MAGIC**: Fixed bytes `[0x56, 0x54]` ("VT")
- **VER**: Protocol version (currently 0x01)
- **TYPE**: Frame type (Hello, Welcome, Data, etc.)
- **FLAGS**: Frame flags (REQ_ACK, REQ_RETRY, etc.)
- **HEADER_LEN**: Total length of all headers in bytes (little-endian)
- **PAYLOAD_LEN**: Length of payload in bytes (big-endian)
- **HEADERS**: Variable-length header section
- **PAYLOAD**: Variable-length payload section
- **CRC**: CRC-32 checksum of entire frame excluding CRC field

### Header Format

Each header follows this structure:

```
[KEY_LEN (1B)] [VALUE_LEN (1B)] [KEY] [VALUE]
```

- **KEY_LEN**: Length of header key in bytes
- **VALUE_LEN**: Length of header value in bytes
- **KEY**: Header key bytes
- **VALUE**: Header value bytes

### Frame Types

- `0x01`: Hello - Initial connection handshake
- `0x02`: Welcome - Server response to Hello
- `0x03`: Data - Application data
- `0x04`: Ack - Acknowledgment
- `0x05`: Nack - Negative acknowledgment
- `0x06`: Ping - Keep-alive
- `0x07`: Pong - Keep-alive response
- `0x08`: Close - Connection termination

### Flags

- `0x01`: REQ_ACK - Request acknowledgment
- `0x02`: REQ_RETRY - Request retry on failure
- `0x04`: URGENT - High priority frame
- `0x08`: COMPRESSED - Payload is compressed
- `0x10`: ENCRYPTED - Payload is encrypted

## Implementation Details

### Core Types

```rust
pub struct Frame {
    pub version: u8,
    pub typ: FrameType,
    pub flags: Flags,
    pub headers: Vec<Header>,
    pub payload: Vec<u8>,
}

pub struct Header {
    pub key: Vec<u8>,
    pub value: Vec<u8>,
}
```

### Encoding Process

1. **Fixed Header**: Write magic, version, type, flags
2. **Header Section**:
   - Calculate total header length
   - Write header length (little-endian)
   - Write payload length (big-endian)
   - Encode each header with key/value lengths
3. **Payload Section**: Write raw payload bytes
4. **CRC Calculation**: Calculate CRC-32 over entire frame (excluding CRC field)
5. **CRC Writing**: Append CRC to end

### Decoding Process

1. **Validation**: Check minimum frame size and magic bytes
2. **Fixed Header**: Parse version, type, flags
3. **Lengths**: Parse header and payload lengths
4. **Header Parsing**:
   - Read key/value lengths for each header
   - Extract header key/value pairs
5. **Payload Extraction**: Read payload bytes
6. **CRC Validation**: Verify CRC matches calculated value

## Current Issues (Step 1)

### 1. CRC Calculation Problems

**Issue**: CRC mismatch between encoded and decoded frames

- Expected: 11892, Got: 40747
- Root cause: CRC calculation includes wrong data range

**Solution**: Ensure CRC is calculated over the correct byte range (entire frame excluding CRC field)

### 2. Payload Corruption

**Issue**: Extra bytes being added to payload during encoding/decoding

- Original: `[84, 104, 105, 115, ...]` ("This is a test payload...")
- Decoded: `[37, 84, 104, 105, 115, ...]` (extra byte 37)

**Root cause**: Incorrect payload length handling or buffer management

### 3. Header Parsing Errors

**Issue**: "Incomplete header value" errors during decoding

- Root cause: Header parsing loop logic is incorrect
- May be reading beyond available data

### 4. Buffer Range Errors

**Issue**: "range end index 11 out of range for slice of length 10"

- Root cause: Attempting to access buffer indices that don't exist
- Need better bounds checking

## Testing Strategy

### Unit Tests

1. **Basic Roundtrip**: Encode → decode → compare
2. **Header Handling**: Frames with various header combinations
3. **Payload Handling**: Frames with different payload sizes
4. **Flag Combinations**: All flag combinations
5. **Frame Types**: All frame type variants
6. **Error Cases**: Malformed frames, incomplete data

### Integration Tests

1. **Large Payloads**: Test with payloads > 1MB
2. **Many Headers**: Test with 100+ headers
3. **Edge Cases**: Empty payloads, empty headers, maximum sizes

## Performance Considerations

- **Zero-copy**: Use `Bytes` for efficient memory management
- **Buffer Reuse**: Reuse buffers when possible
- **Batch Processing**: Handle multiple frames efficiently
- **Memory Limits**: Enforce maximum frame size limits

## Security Considerations

- **Input Validation**: Validate all input data
- **Size Limits**: Prevent memory exhaustion attacks
- **CRC Validation**: Ensure data integrity
- **Version Checking**: Handle protocol version mismatches

## Next Steps

After fixing the current issues:

1. **Performance Optimization**: Profile and optimize encoding/decoding
2. **Error Handling**: Improve error messages and recovery
3. **Documentation**: Add more detailed API documentation
4. **Benchmarks**: Create performance benchmarks
5. **Fuzzing**: Add fuzz testing for robustness

## Files Created

- `src/types.rs` - Core type definitions
- `src/frame.rs` - Frame encoding/decoding logic
- `src/codec.rs` - Tokio codec integration
- `src/lib.rs` - Library exports
- `tests/frame_tests.rs` - Comprehensive test suite

## Dependencies

- `bytes` - Efficient byte buffer management
- `byteorder` - Endianness handling
- `thiserror` - Error type definitions
- `bitflags` - Flag bit manipulation
- `crc-any` - CRC calculation
- `tokio-util` - Async I/O integration
