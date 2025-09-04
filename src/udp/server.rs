//! UDP server implementation for VSTP

use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::UdpSocket;
use tokio::sync::Mutex;
use tracing::{debug, error, info, warn};

use crate::frame::try_decode_frame;
use crate::types::{Flags, Frame, FrameType, Header, VstpError, VSTP_VERSION};
use crate::udp::reassembly::{extract_fragment_info, ReassemblyManager, MAX_DATAGRAM_SIZE};

/// Configuration for UDP server
#[derive(Debug, Clone)]
pub struct UdpServerConfig {
    /// Whether to use CRC validation
    pub use_crc: bool,
    /// Whether to allow fragmentation
    pub allow_frag: bool,
    /// Maximum number of concurrent reassembly sessions
    pub max_reassembly_sessions: usize,
}

impl Default for UdpServerConfig {
    fn default() -> Self {
        Self {
            use_crc: true,
            allow_frag: true,
            max_reassembly_sessions: 1000,
        }
    }
}

/// VSTP UDP Server
pub struct VstpUdpServer {
    socket: UdpSocket,
    #[allow(dead_code)]
    config: UdpServerConfig,
    reassembly: ReassemblyManager,
    #[allow(dead_code)]
    next_session_id: Arc<Mutex<u128>>,
}

impl VstpUdpServer {
    /// Create a new UDP server bound to the specified address
    pub async fn bind(addr: &str) -> Result<Self, VstpError> {
        let socket = UdpSocket::bind(addr).await?;
        info!("VSTP UDP server bound to {}", addr);

        Ok(Self {
            socket,
            config: UdpServerConfig::default(),
            reassembly: ReassemblyManager::new(),
            next_session_id: Arc::new(Mutex::new(1)),
        })
    }

    /// Create a new UDP server with custom configuration
    pub async fn bind_with_config(addr: &str, config: UdpServerConfig) -> Result<Self, VstpError> {
        let socket = UdpSocket::bind(addr).await?;
        info!("VSTP UDP server bound to {} with custom config", addr);

        Ok(Self {
            socket,
            config,
            reassembly: ReassemblyManager::new(),
            next_session_id: Arc::new(Mutex::new(1)),
        })
    }

    /// Get the local address this server is bound to
    pub fn local_addr(&self) -> Result<SocketAddr, VstpError> {
        self.socket.local_addr().map_err(|e| VstpError::Io(e))
    }

    /// Run the server with a frame handler
    pub async fn run<F, Fut>(self, handler: F) -> Result<(), VstpError>
    where
        F: Fn(SocketAddr, Frame) -> Fut + Send + Sync + Clone + 'static,
        Fut: std::future::Future<Output = ()> + Send,
    {
        info!("VSTP UDP server starting...");

        let mut buf = vec![0u8; MAX_DATAGRAM_SIZE * 2]; // Extra space for headers

        loop {
            match self.socket.recv_from(&mut buf).await {
                Ok((len, from_addr)) => {
                    let data = &buf[..len];
                    debug!("Received {} bytes from {}", len, from_addr);

                    // Handle the frame
                    if let Err(e) = self.handle_frame(from_addr, data, &handler).await {
                        error!("Error handling frame from {}: {}", from_addr, e);
                    }
                }
                Err(e) => {
                    error!("Error receiving UDP packet: {}", e);
                }
            }
        }
    }

    /// Handle a received frame
    async fn handle_frame<F, Fut>(
        &self,
        from_addr: SocketAddr,
        data: &[u8],
        handler: &F,
    ) -> Result<(), VstpError>
    where
        F: Fn(SocketAddr, Frame) -> Fut + Send + Sync + Clone,
        Fut: std::future::Future<Output = ()> + Send,
    {
        // Try to decode the frame
        let mut buf = bytes::BytesMut::from(data);
        let frame = match try_decode_frame(&mut buf, 65536) {
            Ok(Some(frame)) => frame,
            Ok(None) => {
                warn!("Incomplete frame received from {}", from_addr);
                return Ok(());
            }
            Err(e) => {
                warn!("Failed to decode frame from {}: {}", from_addr, e);
                return Ok(());
            }
        };

        // Check if this is a fragmented frame
        if let Some(fragment) = extract_fragment_info(&frame) {
            // Handle fragmentation
            if let Some(assembled_data) = self.reassembly.add_fragment(from_addr, fragment).await? {
                // Reassemble the complete frame
                let mut complete_frame = frame;
                complete_frame.payload = assembled_data;
                // Remove fragment headers
                complete_frame.headers.retain(|h| {
                    h.key != b"frag-id" && h.key != b"frag-index" && h.key != b"frag-total"
                });

                // Handle the complete frame
                self.process_frame(from_addr, complete_frame, handler)
                    .await?;
            } else {
                debug!("Fragment received from {}, waiting for more", from_addr);
            }
        } else {
            // Handle the complete frame
            self.process_frame(from_addr, frame, handler).await?;
        }

        Ok(())
    }

    /// Process a complete frame
    async fn process_frame<F, Fut>(
        &self,
        from_addr: SocketAddr,
        frame: Frame,
        handler: &F,
    ) -> Result<(), VstpError>
    where
        F: Fn(SocketAddr, Frame) -> Fut + Send + Sync + Clone,
        Fut: std::future::Future<Output = ()> + Send,
    {
        // Check if this frame requires an ACK
        if frame.flags.contains(Flags::REQ_ACK) {
            if let Some(msg_id) = self.extract_msg_id(&frame) {
                // Send ACK
                if let Err(e) = self.send_ack(msg_id, from_addr).await {
                    warn!("Failed to send ACK to {}: {}", from_addr, e);
                }
            }
        }

        // Call the user handler
        handler(from_addr, frame).await;
        Ok(())
    }

    /// Extract message ID from frame headers
    fn extract_msg_id(&self, frame: &Frame) -> Option<u64> {
        for header in &frame.headers {
            if header.key == b"msg-id" {
                if let Ok(msg_id) = std::str::from_utf8(&header.value).ok()?.parse::<u64>() {
                    return Some(msg_id);
                }
            }
        }
        None
    }

    /// Send an ACK for a received message
    async fn send_ack(&self, msg_id: u64, dest: SocketAddr) -> Result<(), VstpError> {
        let ack_frame = Frame {
            version: VSTP_VERSION,
            typ: FrameType::Ack,
            flags: Flags::empty(),
            headers: vec![Header {
                key: b"msg-id".to_vec(),
                value: msg_id.to_string().into_bytes(),
            }],
            payload: Vec::new(),
        };

        let encoded = crate::frame::encode_frame(&ack_frame)?;
        self.socket.send_to(&encoded, dest).await?;
        debug!("Sent ACK for message {} to {}", msg_id, dest);
        Ok(())
    }

    /// Get the number of active reassembly sessions
    pub async fn reassembly_session_count(&self) -> usize {
        self.reassembly.session_count().await
    }
}
