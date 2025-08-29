Important defaults:

Default port: 6969 (TCP & UDP)

TCP mode: TLS 1.3 mandatory (rustls)

UDP mode: no TLS v0.1 (CRC/ACK optional)

Step 1 — Project scaffolding + frame/types + encoder/decoder (the wire format)

Goal: Define the single source of truth for the VSTP wire format and implement robust encode/decode routines (unit-tested). This makes the protocol stable before networking.

Tasks

Create crate:

cargo new vstp-core --lib
cd vstp-core


Implement protocol constants and in-memory types.

Implement encode_frame and decode_frame that implement the official frame:

[MAGIC (2B)] [VER (1B)] [TYPE (1B)] [FLAGS (1B)]
[HDR_LEN (2B LE)] [PAY_LEN (4B BE)] [HEADERS...] [PAYLOAD...]
[optional CRC16 over HEADERS|PAYLOAD if FLAGS.CRC=1]


Implement K/V header encoding: [KLEN (1B)] [KEY] [VLEN (2B LE)] [VALUE] ...

Make decoder tolerant to partial buffers (return Ok(None) if more bytes required).

Add unit tests for happy path + malformed frames.

Files to create
src/
  lib.rs          // re-exports + crate-level docs
  types.rs        // Frame, Header, FrameType, Flags, VstpError
  frame.rs        // encode_frame, decode_frame
  codec.rs        // tokio codec helpers (used in Step 2)
tests/
  frame_tests.rs

Dependencies (Cargo.toml)

bytes

byteorder

thiserror

bitflags

crc-any or crc16

(dev) rand for test vectors

Key APIs (signatures)
// types.rs
pub struct Header { pub key: Vec<u8>, pub value: Vec<u8> }
pub struct Frame {
  pub version: u8,
  pub typ: FrameType,
  pub flags: Flags,
  pub headers: Vec<Header>,
  pub payload: Vec<u8>,
}

// frame.rs
pub fn encode_frame(frame: &Frame) -> Result<bytes::Bytes, VstpError>;
pub fn try_decode_frame(buf: &mut bytes::BytesMut) -> Result<Option<Frame>, VstpError>;

Tests

Roundtrip tests: encode -> decode == original (various header counts, empty payload, large payload).

Malformed frame tests (bad header lengths, mismatched payload length).

CRC positive/negative vectors.

Commands
cargo test

Acceptance criteria

All unit tests pass.

try_decode_frame returns Ok(None) for incomplete buffers.

Malformed conditions trigger VstpError::Protocol.

Step 2 — Async TCP client & server + framed transport (plain TCP; no TLS yet)

Goal: Hook the frame codec into an async TCP transport using Tokio; implement a simple server and client (spawnable examples). Verify framed reads/writes across real sockets.

Tasks

Add async runtime and tokio codec.

Implement tokio_util::codec::{Decoder, Encoder} wrapper that uses try_decode_frame and encode_frame.

Implement a connection handler loop (spawned per-connection) that reads frames and calls user handler callback.

Implement a VstpTcpClient with async connect, send, recv, close.

Provide example binaries: examples/tcp_server.rs and examples/tcp_client.rs (client sends HELLO -> DATA -> BYE, server echoes or replies WELCOME).

Add integration tests: start a server task within test and use the client to roundtrip frames.

Files to create / update
src/tcp/
  mod.rs
  server.rs       // VstpTcpServer
  client.rs       // VstpTcpClient
src/tokio_codec.rs // VstpFrameCodec implementing Encoder/Decoder
examples/
  tcp_server.rs
  tcp_client.rs

Dependencies (Cargo.toml)

tokio (full features: rt-multi-thread, net)

tokio-util

tracing (for logs)

futures (optional helpers)

Key APIs (signatures)
// tcp/client.rs
pub struct VstpTcpClient { /* internal framed sink/stream */ }
impl VstpTcpClient {
  pub async fn connect(addr: &str) -> Result<Self, VstpError>;
  pub async fn send(&self, frame: Frame) -> Result<(), VstpError>;
  pub async fn recv(&mut self) -> Result<Option<Frame>, VstpError>;
  pub async fn close(&mut self) -> Result<(), VstpError>;
}

// tcp/server.rs
pub struct VstpTcpServer { /* bind + run */ }
impl VstpTcpServer {
  pub async fn bind(addr: &str) -> Result<Self, VstpError>;
  pub async fn run<F, Fut>(self, handler: F) -> Result<(), VstpError>
    where F: Fn(SessionId, Frame) -> Fut + Send + Sync + 'static,
          Fut: Future<Output = ()> + Send;
}

Tests

Integration test: server accepts client; client sends HELLO -> server replies WELCOME; client receives WELCOME.

Partial read test (simulate split writes).

Commands
cargo run --example tcp_server
# in another terminal
cargo run --example tcp_client

Acceptance criteria

Example server and client can exchange frames and exit cleanly.

Codec handles partial TCP segments and re-assembly.

No panics; proper error propagation.

Step 3 — TLS integration for TCP (secure-by-default)

Goal: Make TCP mode always TLS 1.3. Use rustls/tokio-rustls. Provide dev-mode self-signed certs and production config options.

Tasks

Add rustls, tokio-rustls, rcgen (dev cert generation), and rustls-pemfile.

Add TLS config: TlsConfig struct supporting:

server cert + private key (PEM or in-memory)

client root CA store or danger_accept_invalid_certs for dev

ALPN list (future)

Update server to wrap accepted TcpStream in TlsAcceptor and client TcpStream in TlsConnector before framing.

Ensure rustls enforces TLS 1.3 and secure ciphers.

Update examples: examples/tcp_server_tls.rs and examples/tcp_client_tls.rs.

Tests:

Valid cert handshake test (self-signed on both ends or client trusts CA).

Invalid cert test (client rejects server when not trusting cert).

Files
src/tcp/tls.rs       // helper: build acceptor/connector from TlsConfig
examples/tcp_server_tls.rs
examples/tcp_client_tls.rs

Dependencies

rustls

tokio-rustls

rcgen (dev use)

rustls-pemfile

Key APIs
pub struct TlsConfig {
  pub domain: String,
  pub root_store: Option<RootCertStore>,
  pub accept_invalid_certs: bool, // dev only
}

pub async fn tls_acceptor_from_files(cert_path: &Path, key_path: &Path) -> Result<...>;
pub async fn tls_connector_from_root(root_pem: &Path) -> Result<...>;

Tests

TLS handshakes succeed and frames are exchanged over the encrypted stream.

Client rejects invalid certs when accept_invalid_certs=false.

Commands
cargo run --example tcp_server_tls
cargo run --example tcp_client_tls

Acceptance criteria

TLS handshake completes and no plain-text frames are observed at application layer.

Certificates are validated by default.

Dev flow: easy generate-test-cert helper available.

Step 4 — UDP mode: CRC, fragmentation, optional ACK/reliability

Goal: Implement UDP transport semantics: datagram I/O, CRC integrity (optional), fragmentation/reassembly for big payloads, and optional REQ_ACK/ACK reliability for critical messages.

Tasks

Implement VstpUdpClient and VstpUdpServer wrapping tokio::net::UdpSocket.

Enforce recommended per-datagram size (e.g., <= 1200 bytes); if larger and allow_frag=true, split into fragments with FLAGS.FRAG=1 and headers frag-id, frag-index, frag-total.

CRC: if FLAGS.CRC set, compute CRC16 over HEADERS|PAYLOAD during encode and append; decode validates and rejects otherwise.

REQ_ACK semantics: if FLAGS.REQ_ACK set OR header msg-id present, receiver replies with ACK frame referencing msg-id. Sender retries with exponential backoff (configurable attempts).

Implement reassembly buffer keyed by (session-id, frag-id), with timeout and memory bounds.

Provide send_with_ack helper that returns Ok(()) only after ACK received or retries exhausted.

Files
src/udp/client.rs
src/udp/server.rs
src/udp/reassembly.rs

Dependencies

tokio (already present)

crc-any (already present)

dashmap or tokio::sync::Mutex<HashMap> for reassembly tables

bytes

Key APIs
pub struct VstpUdpClient { /* socket, cfg */ }
impl VstpUdpClient {
  pub async fn bind(local: &str) -> Result<Self, VstpError>;
  pub async fn send(&self, frame: Frame, dest: SocketAddr) -> Result<(), VstpError>;
  pub async fn send_with_ack(&self, frame: Frame, dest: SocketAddr) -> Result<(), VstpError>;
  pub async fn recv(&mut self) -> Result<(Frame, SocketAddr), VstpError>;
}

Tests

UDP echo (small datagram) test.

Fragmentation+reassembly: send a payload requiring 3 fragments; assert reassembled payload matches original.

CRC failure: corrupt a frame mid-flight and assert receiver rejects it.

ACK retry: simulate lost ACKs (drop first N) and verify client retries then succeeds.

Commands
cargo run --example udp_server
cargo run --example udp_client

Acceptance criteria

Library sends and receives UDP frames and correctly handles CRC and fragment reassembly.

send_with_ack returns success when ACK arrives; retries and fails gracefully otherwise.

Memory/time limits applied to reassembly caches to prevent DoS.

Step 5 — WASM bindings (npm wrapper), CLI tools, docs, CI & publishing

Goal: Make VSTP usable by JavaScript/TS developers via npm, provide developer CLI tools, CI, and publishable artifacts.

Tasks

Decide binding strategy:

Recommended: Use Rust for framing/crypto logic compiled to WASM (via wasm-pack) and expose high-level functions; keep socket ownership in Node (use Node net module for sockets) OR target nodejs in wasm-pack and use napi if native.

Simpler alternative: expose encode/decode + helpers via WASM and write a thin TS wrapper that handles networking on Node.

Implement wasm-bindgen exports for:

encode_frame, decode_frame (maybe async wrappers)

Optionally frame builder helpers.

Create Node package structure:

npm/
  package.json
  index.js (loads wasm)
  lib/vstp-client.js (TS wrapper exposing VSTPClient)


Publish npm package: @vishu/vstp-core or vstp.

Build CLI: src/bin/vstp-cli.rs with commands:

server --tcp --tls / client --send

inspect <hexfile|pcap>

Docs & spec:

Finalize docs/spec.md and README.md

Add examples in examples/node_client.js

CI (GitHub Actions):

ci.yml runs: cargo fmt -- --check, cargo clippy, cargo test, wasm-pack build, node tests for the wrapper.

publish.yml for crates.io (manual token) and npm (manual token).

Release: cargo publish and npm publish (after version bump).

Files
wasm/               // wasm build config
npm/                // JS wrapper and package
src/bin/vstp-cli.rs // CLI
docs/               // README + spec + examples
.github/workflows/

Dependencies / Tools

wasm-pack, wasm-bindgen, wasm-bindgen-futures

node / npm for testing npm package

napi-rs (optional alternative to wasm for Node native addon)

gh-actions setup

Key APIs (JS)
// high-level TypeScript wrapper
const client = await VSTP.connect({ host: "127.0.0.1", port: 6969, transport: "tcp" });
await client.send({ type: "DATA", headers: { "content-type": "text/plain" }, payload: new TextEncoder().encode("hi") });
client.on("frame", (frame) => { ... });

Tests

Node integration test using npm wrapper to connect to Rust TLS server running in CI container.

WASM unit tests (via wasm-pack test node).

Commands
# build wasm
wasm-pack build --target nodejs -o ./npm/pkg

# run node example
node npm/examples/node_client.js

Acceptance criteria

npm package can be installed and used to build/send frames from Node (connects to the Rust TLS server).

CLI vstp-cli server and vstp-cli client usable locally.

CI passes on PRs and publishes artifacts on release.

Cross-cutting details (apply across all steps)

Configuration struct:

pub struct VstpConfig {
  pub addr: String,
  pub transport: Transport, // Tcp or Udp
  pub tls: Option<TlsConfig>,
  pub keepalive: Option<Duration>,
  pub allow_udp_frag: bool,
  pub use_crc_udp: bool,
  pub max_frame_size: usize,
  pub retries: usize,
}


Logging: tracing everywhere, support RUST_LOG.

Limits: enforce max_frame_size, max_headers, max_header_len.

Security: TLS by default on TCP, validate certs by default.

Docs: keep docs/spec.md and in-code doc comments in sync; include wire format examples and a hex-dump example.

Backpressure: use bounded mpsc channels in client send path; if full, return Backpressure error.

Final acceptance (end-to-end)

When all five steps are completed you will have:

A spec-compliant Rust crate providing VSTP framing, TLS-enabled TCP transport, UDP transport with optional CRC/ACK/frag, and a stable async API.

Example binaries (server/client) for TCP-TLS and UDP.

WASM bindings and an npm wrapper that JS/TS developers can use.

CLI tools for development and inspection, CI pipelines, and publishable artifacts.