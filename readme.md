
# 🌐 VSTP – Vishu’s Secure Transfer Protocol

## Rust Core Library – Design & Implementation Guide (v0.1)

**Status:** Draft
**Default port:** `6969` (TCP & UDP)
**Transports:** TCP (TLS 1.3), UDP (plaintext for v0.1)
**License:** MIT

---

## 1) Vision & Goals

**VSTP** is a general-purpose, binary, extensible application-layer protocol designed to be:

* **Secure by default** on TCP (TLS 1.3).
* **Fast** on UDP (no TLS initially).
* **Minimal but extensible** with binary headers.
* **Easy to implement** across languages.

We provide a **Rust reference implementation** as the gold-standard core, with optional **WASM bindings** for Node/TS.

---

## 2) High-Level Architecture

```
+-------------------------------+          +------------------------------+
|           Application         |          |          Application         |
|  (chat, files, IoT, custom)   |          |  (chat, files, IoT, custom)  |
+-------------------------------+          +------------------------------+
|           VSTP API            |  <-----> |           VSTP API           |
|  (Rust crate / npm via WASM)  |          |  (Rust crate / npm via WASM) |
+-------------------------------+          +------------------------------+
|       Framing & Headers       |  <-----> |       Framing & Headers      |
+-------------------------------+          +------------------------------+
|   TCP(+TLS) or UDP Transport  |  <-----> |   TCP(+TLS) or UDP Transport |
+-------------------------------+          +------------------------------+
|           OS Sockets          |          |           OS Sockets         |
+-------------------------------+          +------------------------------+
```

* **TCP mode**: reliable + encrypted (TLS via `rustls`).
* **UDP mode**: connectionless + fast (no TLS in v0.1).
* **Same frame format** on both (fragmentation only needed for UDP/large payloads).

---

## 3) Frame & Header Specification (v1)

### 3.1 Frame Layout

```
+------------+----------+----------+--------------+-----------------+-----------------+
| MAGIC (2B) | VER (1B) | TYPE(1B) | FLAGS (1B)   | HDR_LEN (2B LE) | PAY_LEN (4B BE) |
+------------+----------+----------+--------------+-----------------+-----------------+
| HEADERS (HDR_LEN bytes)                                                 |
+----------------------------------------------------------------------------+
| PAYLOAD (PAY_LEN bytes)                                                   |
+----------------------------------------------------------------------------+
| OPTIONAL: CHECKSUM (2B CRC16)  [UDP-only if FLAGS.CRC=1]                  |
+----------------------------------------------------------------------------+
```

**Fields**

* **MAGIC**: `0x56 0x54` (“VT”) to identify VSTP.
* **VER**: `0x01` (protocol version).
* **TYPE**: one-byte message kind (see §3.4).
* **FLAGS** (bitfield, see §3.3).
* **HDR\_LEN**: little-endian `u16` for header section length (0–65535).
* **PAY\_LEN**: big-endian `u32` for payload length (0–4,294,967,295).
* **HEADERS**: concatenated binary K/V entries (see §3.2).
* **PAYLOAD**: raw bytes (UTF-8 text, JSON, binary, etc).
* **CHECKSUM**: `CRC16-IBM` over entire frame **excluding** MAGIC/VER/TYPE/FLAGS/HDR\_LEN/PAY\_LEN?
  → **Decision**: CRC covers `HEADERS | PAYLOAD` **only** (simpler & stable for both). Present when `FLAGS.CRC=1` (recommended for UDP).

### 3.2 Header Encoding (K/V sequence)

Each header is `[KLEN(1B)][KEY(KLEN)][VLEN(2B LE)][VALUE(VLEN)]`.

Example (2 headers):

```
04 "from"  0003 "bob"
02 "to"    0003 "ann"
```

**Key rules**

* `KLEN` ∈ \[1,255].
* Keys are ASCII lowercase `[a-z0-9-_]` recommended.
* Values are arbitrary bytes (UTF-8 recommended for strings).
* Repeated keys allowed (e.g., multi-valued headers).
* Unknown headers MUST be ignored.

### 3.3 Flags (1 byte)

```
bit 7  bit 6  bit 5  bit 4  bit 3  bit 2       bit 1     bit 0
RES    RES    RES    COMP   FRAG   RESERVED    CRC       REQ_ACK
```

* **REQ\_ACK (b0)**: sender requests an ACK frame.
* **CRC (b1)**: CRC16 present (UDP recommended).
* **FRAG (b3)**: this frame is a fragment (see §6.3).
* **COMP (b4)**: payload compressed (e.g., LZ4) (v0.2+).
* Other bits reserved, must be zero.

### 3.4 Message Types (TYPE)

| Hex  | Name    | Direction       | Notes                                           |
| ---- | ------- | --------------- | ----------------------------------------------- |
| `01` | HELLO   | Client → Server | Start of session; includes client meta headers. |
| `02` | WELCOME | Server → Client | Server accept; may include server meta.         |
| `03` | DATA    | Both            | Application data.                               |
| `04` | PING    | Both            | Keepalive request.                              |
| `05` | PONG    | Both            | Keepalive response.                             |
| `06` | BYE     | Both            | Graceful close.                                 |
| `07` | ACK     | Both            | Acknowledgement (for REQ\_ACK usage).           |
| `08` | ERR     | Both            | Error frame; body = machine-readable code.      |

**Common headers** (optional, by convention):

* `content-type`: e.g., `application/json`, `text/plain`, `application/octet-stream`
* `session-id`: opaque binary/UUID
* `msg-id`: sender’s message UUID
* `corr-id`: correlation id for request/response pairing
* `encoding`: `utf8` / `binary`
* `filename`: when sending files
* `chunk`: chunk index/total for file streams (or use FRAG)

---

## 4) Connection Lifecycles

### 4.1 TCP (TLS)

1. **TCP connect** to `ip:6969`.
2. **TLS 1.3 handshake** with server certificate (via `rustls`).
3. **HELLO** (client → server) with headers like:

   * `client-name`, `client-version`
   * `app-name`, `session-id` (optional)
4. **WELCOME** (server → client) with headers like:

   * `server-name`, `server-version`
5. **Exchange** DATA, PING/PONG, ACK, ERR frames as needed.
6. **BYE** (either side), then TCP/TLS close.

> **Note**: TLS provides integrity + confidentiality. CRC is **not** needed over TCP/TLS (ignore `FLAGS.CRC`).

### 4.2 UDP (No TLS in v0.1)

1. Client sends **HELLO** to `ip:6969` (include `session-id` header).
2. Server replies **WELCOME** (echo/assign `session-id`).
3. Client sends **DATA** (optionally `FLAGS.CRC=1`, `REQ_ACK` if reliability needed).
4. Server optionally acknowledges via **ACK**.
5. PING/PONG for liveness if desired.
6. No formal close—optionally send **BYE**.

> **Note**: For NAT traversal, the server identifies client by `(ip:port, session-id)`. Timeouts expire sessions.

---

## 5) Error Handling

### 5.1 ERR Payload

ERR payload is a small binary or JSON blob with:

* `code` (u16): machine code
* `msg` (string): human hint
* Optional headers: `retry-after`, `details`

### 5.2 Standard Error Codes

| Code  | Name               | When                         |
| ----- | ------------------ | ---------------------------- |
| 0x001 | INVALID\_VERSION   | VER not supported            |
| 0x002 | INVALID\_TYPE      | Unknown TYPE                 |
| 0x003 | BAD\_LENGTH        | PAY\_LEN/HDR\_LEN mismatch   |
| 0x004 | BAD\_HEADERS       | Malformed K/V sequence       |
| 0x005 | AUTH\_REQUIRED     | (Future) server demands auth |
| 0x006 | PERMISSION\_DENIED | (Future) unauthorized action |
| 0x007 | RATE\_LIMITED      | Backoff required             |
| 0x008 | INTERNAL\_ERROR    | Unhandled error              |
| 0x009 | TLS\_ERROR         | (TCP) TLS failed             |

---

## 6) UDP Specifics

### 6.1 Preferred Best-Effort

* Small, atomic messages (≤ 1200 bytes suggested) to avoid fragmentation at IP level.
* For larger data, either:

  * move to TCP, or
  * use **VSTP fragmentation** (§6.3).

### 6.2 Optional Reliability Aids

* **REQ\_ACK** flag to request **ACK** on DATA.
* App-level retries with exponential backoff.
* **CRC** flag for per-frame integrity (CRC16 over `HEADERS|PAYLOAD`).

### 6.3 VSTP Fragmentation (UDP)

If `FLAGS.FRAG=1`, include headers:

* `frag-id` (u64) – unique per message
* `frag-index` (u16) – 0-based
* `frag-total` (u16) – total fragments

Receiver buffers fragments by `(session-id, frag-id)` until all `frag-total` arrive (or timeout), then reassembles.

---

## 7) Rust Crate Design

### 7.1 Crate Layout

```
vstp-core/
 ├─ Cargo.toml
 └─ src/
    ├─ lib.rs
    ├─ frame.rs         # encode/decode, headers, CRC
    ├─ types.rs         # enums, flags, errors
    ├─ tcp/
    │   ├─ client.rs    # TLS client (rustls)
    │   └─ server.rs    # TLS server (rustls), accept loop
    ├─ udp/
    │   ├─ client.rs
    │   └─ server.rs
    ├─ util/
    │   ├─ codec.rs     # length delimited, io helpers
    │   └─ time.rs      # timers, backoff
    └─ config.rs        # VstpConfig, tuning knobs
```

### 7.2 Dependencies (suggested)

* `tokio` (async runtime)
* `tokio-util` (codec helpers)
* `rustls`, `tokio-rustls` (TLS)
* `bytes` (buffer handling)
* `crc` or `crc16` (CRC16-IBM)
* `thiserror` (error types)
* `tracing` (structured logs)
* `lz4_flex` (v0.2+ for compression)
* `serde`, `serde_json` (examples/tests)

### 7.3 Public API (high-level sketch)

```rust
// config.rs
pub struct VstpConfig {
    pub transport: Transport,      // Transport::Tcp | Transport::Udp
    pub addr: String,              // "host:port"
    pub tls: Option<TlsConfig>,    // only for TCP
    pub keepalive: Option<Duration>,
    pub use_crc_udp: bool,
    pub max_frame_size: usize,     // defense-in-depth
    pub recv_queue: usize,         // channel bounds
    // future: compression, auth hooks, etc.
}

pub enum Transport { Tcp, Udp }

pub struct TlsConfig {
    pub domain: String,            // SNI / cert validation
    pub root_certs: RootStore,     // or PEM path(s)
}

// client.rs
pub struct VstpClient { /* ... */ }

impl VstpClient {
    pub async fn connect(cfg: VstpConfig) -> Result<Self, VstpError>;
    pub async fn send(&self, frame: Frame) -> Result<(), VstpError>;
    pub async fn recv(&self) -> Result<Frame, VstpError>;
    pub async fn close(&self) -> Result<(), VstpError>;
}

// server.rs
pub struct VstpServer { /* ... */ }

impl VstpServer {
    pub async fn bind(cfg: VstpConfig) -> Result<Self, VstpError>;
    pub async fn run<F>(self, on_frame: F) -> Result<(), VstpError>
      where F: Fn(SessionId, Frame) -> ServerAction + Send + Sync + 'static;
}

// types.rs
pub type SessionId = u128;

#[repr(u8)]
pub enum FrameType { Hello=0x01, Welcome=0x02, Data=0x03, Ping=0x04, Pong=0x05, Bye=0x06, Ack=0x07, Err=0x08 }

bitflags::bitflags! {
  pub struct Flags: u8 {
    const REQ_ACK = 0b0000_0001;
    const CRC     = 0b0000_0010;
    const FRAG    = 0b0001_0000;
    const COMP    = 0b0010_0000;
  }
}

pub struct Header { pub key: Vec<u8>, pub value: Vec<u8> }

pub struct Frame {
    pub version: u8,        // 0x01
    pub typ: FrameType,
    pub flags: Flags,
    pub headers: Vec<Header>,
    pub payload: Vec<u8>,
}

// errors.rs
#[derive(thiserror::Error, Debug)]
pub enum VstpError {
    #[error("io: {0}")] Io(#[from] std::io::Error),
    #[error("tls: {0}")] Tls(String),
    #[error("protocol: {0}")] Protocol(String),
    #[error("timeout")] Timeout,
    #[error("closed")] Closed,
}
```

### 7.4 Framing/Codec (TCP)

* We read from a **TLS stream** (tokio-rustls) and parse frames:

  * Read fixed header (MAGIC..PAY\_LEN).
  * Read `HDR_LEN` bytes of headers → parse K/V sequence.
  * Read `PAY_LEN` bytes of payload.
  * If `FLAGS.CRC`, compute & validate CRC16 over `HEADERS|PAYLOAD` (TCP won’t set it in v0.1).

**Defense-in-depth**:

* Reject frames exceeding `max_frame_size`.
* Reject malformed headers (bounds check KLEN/VLEN).
* Time out slowloris (idle read timeouts).

### 7.5 UDP Loop

* Each `send(frame)` becomes **one datagram** if size ≤ MTU budget (\~1200 bytes recommended).
* For larger payloads, either:

  * app chooses TCP, or
  * library splits into fragments if `cfg.allow_udp_frag = true` (v0.1 can leave fragmentation to app; v0.2 can add automatic fragmentation).

---

## 8) Keepalive & Flow Control

* **Keepalive**: if `keepalive=Some(d)`, send **PING** after `d` idle, expect **PONG** within `d/2`; else mark dead.
* **Backpressure**:

  * Use bounded channels for outbound queue.
  * If full, `send` returns `WouldBlock`/`Backpressure` error (caller can await space).
* **Auto-reconnect** (optional v0.2):

  * For TCP: attempt reconnect with exponential backoff, re-HELLO.

---

## 9) Security

* **TCP**: Enforce **TLS 1.3** (via `rustls`), validate server certs (SNI/roots).

  * Optionally support **mTLS** (client certs) later.
* **UDP**: No encryption in v0.1. Strongly recommend **not** sending secrets.

  * CRC optional for integrity (tamper-detection is weak; it’s for accidental corruption, not attackers).
* **Header/payload limits** to prevent memory abuse.
* **Zero-copy** where possible; avoid storing unnecessary copies of large payloads.

---

## 10) Compression (v0.2+)

* If enabled, set `FLAGS.COMP`.
* Add header `comp: lz4`.
* Compress **payload only** (headers remain uncompressed).
* Decompress on receive when `FLAGS.COMP` is set.

---

## 11) CLI Tools (for developers)

* `vstp-cli server --tcp 0.0.0.0:6969`
* `vstp-cli client --tcp 127.0.0.1:6969 --send "hello"`
* `vstp-cli echo --udp 127.0.0.1:6969`
* `vstp-inspect <pcap|hex>` to pretty-print VSTP frames from a hex dump.

These help validate interop & load testing.

---

## 12) Testing Strategy

* **Unit tests**: frame encode/decode, header parsing, CRC vectors.
* **Property tests**: fuzz malformed frames (cargo-fuzz later).
* **Integration tests**: TCP TLS round-trip, UDP echo, PING/PONG.
* **Soak tests**: long-lived connections, backpressure, reconnects.

---

## 13) Performance Targets (initial)

* TCP small messages (<= 1KB): < 1 ms p99 on localhost per RTT.
* UDP echo (<= 512B): wire to wire < 0.5 ms p99 on localhost.
* Zero allocations on hot paths where practical (reuse buffers via `bytes`).

---

## 14) Versioning & Compatibility

* **Spec version** in `VER` field (`0x01` now).
* Minor changes: additive (new headers/flags/types), MUST be ignored safely by older peers.
* Breaking changes → bump `VER`, negotiate in HELLO/WELCOME.

---

## 15) Example Flows (ASCII)

### 15.1 TCP Chat Message

```
Client                           Server
  |  TLS connect:6969              |
  |------------------------------->|
  |  HELLO                         |
  |------------------------------->|
  |                 WELCOME        |
  |<-------------------------------|
  |  DATA "hi" (REQ_ACK)           |
  |------------------------------->|
  |                 ACK(msg-id)    |
  |<-------------------------------|
  |  BYE                           |
  |------------------------------->|
  |               close            |
```

### 15.2 UDP Move Command (No TLS)

```
Client                                 Server
  | HELLO (session-id=abc)               |
  |------------------------------------->|
  |                    WELCOME(abc)      |
  |<-------------------------------------|
  | DATA "move:5,10" (CRC, REQ_ACK)      |
  |------------------------------------->|
  |                   ACK(corr-id)       |
  |<-------------------------------------|
```

---

## 16) Example: Minimal Rust Usage

```rust
use vstp_core::{VstpClient, VstpConfig, Transport, Frame, FrameType, Flags};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cfg = VstpConfig {
        transport: Transport::Tcp,
        addr: "127.0.0.1:6969".into(),
        tls: Some(default_tls("localhost")?),
        keepalive: Some(std::time::Duration::from_secs(30)),
        use_crc_udp: false,
        max_frame_size: 8 * 1024 * 1024,
        recv_queue: 1024,
    };

    let client = VstpClient::connect(cfg).await?;

    let mut frame = Frame::new(FrameType::Data);
    frame.flags |= Flags::REQ_ACK;
    frame.header_str("content-type", "text/plain");
    frame.payload = b"hello via VSTP".to_vec();

    client.send(frame).await?;

    let reply = client.recv().await?;
    println!("got: {:?} {:?}", reply.typ, String::from_utf8_lossy(&reply.payload));

    client.close().await?;
    Ok(())
}
```

---

## 17) Node/TS via WASM (outline)

* Build with `wasm-pack`.
* Publish `@vstp/core` to npm.
* TS wrapper exposes:

  * `connect({ host, port, transport })`
  * `send({ type, headers, payload, flags })`
  * `on('frame', fn)`

> Browsers can only use UDP via WebRTC or TCP via WebSocket proxy. For browser support, provide a **VSTP↔WebSocket gateway** (separate project).

---

## 18) Future Work

* **UDP security**: lightweight authenticated encryption (e.g., XChaCha20-Poly1305 + X25519 key exchange) → “VSTLS-U”.
* **Multiplexing**: stream IDs inside DATA frames (HTTP/2-like).
* **Auth**: tokens, mTLS, SASL-like flows.
* **Compression**: LZ4/deflate payload compression.
* **Observability**: metrics, tracing spans, Wireshark dissector.

---

## 19) Summary

* **VSTP** delivers a **compact, binary, extensible** protocol with **TLS-by-default on TCP** and **fast UDP**.
* The **Rust core** provides: framing, headers, TLS, TCP/UDP clients/servers, keepalive, and a clean async API.
* The spec is simple enough for others to re-implement, but powerful enough for real systems (chat, files, IoT, realtime control).

---

