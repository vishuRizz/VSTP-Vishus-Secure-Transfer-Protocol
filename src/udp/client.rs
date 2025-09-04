//! UDP client implementation for VSTP

use std::net::SocketAddr;
use std::time::{Duration, Instant};
use tokio::net::UdpSocket;
use tokio::time::timeout;
use tracing::{debug, error, info, warn};

use crate::frame::encode_frame;
use crate::types::{Flags, Frame, FrameType, Header, VstpError, VSTP_VERSION};
use crate::udp::reassembly::{fragment_payload, ReassemblyManager, MAX_DATAGRAM_SIZE};

/// Configuration for UDP client
#[derive(Debug, Clone)]
pub struct UdpConfig {
    /// Maximum number of retry attempts for ACK requests
    pub max_retries: usize,
    /// Initial retry delay
    pub retry_delay: Duration,
    /// Maximum retry delay (exponential backoff cap)
    pub max_retry_delay: Duration,
    /// Timeout for ACK responses
    pub ack_timeout: Duration,
    /// Whether to use CRC validation
    pub use_crc: bool,
    /// Whether to allow fragmentation
    pub allow_frag: bool,
}

impl Default for UdpConfig {
    fn default() -> Self {
        Self {
            max_retries: 3,
            retry_delay: Duration::from_millis(100),
            max_retry_delay: Duration::from_secs(5),
            ack_timeout: Duration::from_secs(2),
            use_crc: true,
            allow_frag: true,
        }
    }
}

/// VSTP UDP Client
pub struct VstpUdpClient {
    socket: UdpSocket,
    config: UdpConfig,
    reassembly: ReassemblyManager,
    next_msg_id: u64,
}

impl VstpUdpClient {
    /// Create a new UDP client bound to the specified local address
    pub async fn bind(local_addr: &str) -> Result<Self, VstpError> {
        let socket = UdpSocket::bind(local_addr).await?;
        info!("VSTP UDP client bound to {}", local_addr);

        Ok(Self {
            socket,
            config: UdpConfig::default(),
            reassembly: ReassemblyManager::new(),
            next_msg_id: 1,
        })
    }

    /// Create a new UDP client with custom configuration
    pub async fn bind_with_config(local_addr: &str, config: UdpConfig) -> Result<Self, VstpError> {
        let socket = UdpSocket::bind(local_addr).await?;
        info!("VSTP UDP client bound to {} with custom config", local_addr);

        Ok(Self {
            socket,
            config,
            reassembly: ReassemblyManager::new(),
            next_msg_id: 1,
        })
    }

    /// Send a frame to the specified destination
    pub async fn send(&self, frame: Frame, dest: SocketAddr) -> Result<(), VstpError> {
        let encoded = encode_frame(&frame)?;

        // Check if we need fragmentation
        if encoded.len() > MAX_DATAGRAM_SIZE && self.config.allow_frag {
            return self.send_fragmented(frame, dest).await;
        }

        // Send as single datagram
        self.socket.send_to(&encoded, dest).await?;
        debug!("Sent frame to {} ({} bytes)", dest, encoded.len());
        Ok(())
    }

    /// Send a frame with ACK reliability
    pub async fn send_with_ack(&mut self, frame: Frame, dest: SocketAddr) -> Result<(), VstpError> {
        let msg_id = self.next_msg_id;
        self.next_msg_id += 1;

        // Add message ID header for ACK tracking
        let mut frame_with_id = frame;
        frame_with_id.headers.push(Header {
            key: b"msg-id".to_vec(),
            value: msg_id.to_string().into_bytes(),
        });

        // Set REQ_ACK flag
        frame_with_id.flags.insert(Flags::REQ_ACK);

        // Try sending with retries
        for attempt in 0..=self.config.max_retries {
            // Send the frame
            self.send(frame_with_id.clone(), dest).await?;

            // Wait for ACK
            match self.wait_for_ack(msg_id, dest).await {
                Ok(_) => {
                    debug!("Received ACK for message {} from {}", msg_id, dest);
                    return Ok(());
                }
                Err(_e) if attempt < self.config.max_retries => {
                    let delay = self.calculate_retry_delay(attempt);
                    warn!(
                        "ACK timeout for message {} (attempt {}/{}), retrying in {:?}",
                        msg_id,
                        attempt + 1,
                        self.config.max_retries + 1,
                        delay
                    );
                    tokio::time::sleep(delay).await;
                }
                Err(e) => {
                    error!(
                        "Failed to receive ACK for message {} after {} attempts: {}",
                        msg_id,
                        self.config.max_retries + 1,
                        e
                    );
                    return Err(e);
                }
            }
        }

        Ok(())
    }

    /// Receive a frame from any source
    pub async fn recv(&mut self) -> Result<(Frame, SocketAddr), VstpError> {
        let mut buf = vec![0u8; MAX_DATAGRAM_SIZE * 2]; // Extra space for headers

        loop {
            let (len, from_addr) = self.socket.recv_from(&mut buf).await?;
            let data = &buf[..len];

            debug!("Received {} bytes from {}", len, from_addr);

            // Try to decode as a complete frame first
            let mut buf = bytes::BytesMut::from(data);
            match crate::frame::try_decode_frame(&mut buf, 65536) {
                Ok(Some(frame)) => {
                    // Check if this is a fragmented frame
                    if let Some(fragment) = crate::udp::reassembly::extract_fragment_info(&frame) {
                        // Handle fragmentation
                        if let Some(assembled_data) =
                            self.reassembly.add_fragment(from_addr, fragment).await?
                        {
                            // Reassemble the complete frame
                            let mut complete_frame = frame;
                            complete_frame.payload = assembled_data;
                            // Remove fragment headers
                            complete_frame.headers.retain(|h| {
                                h.key != b"frag-id"
                                    && h.key != b"frag-index"
                                    && h.key != b"frag-total"
                            });
                            return Ok((complete_frame, from_addr));
                        } else {
                            // Fragment received, continue waiting for more
                            continue;
                        }
                    } else {
                        // Complete frame received
                        return Ok((frame, from_addr));
                    }
                }
                Ok(None) => {
                    // Incomplete frame, continue waiting
                    continue;
                }
                Err(e) => {
                    warn!("Failed to decode frame from {}: {}", from_addr, e);
                    continue;
                }
            }
        }
    }

    /// Send a fragmented frame
    async fn send_fragmented(&self, frame: Frame, dest: SocketAddr) -> Result<(), VstpError> {
        let encoded = encode_frame(&frame)?;
        let frag_id = (self.next_msg_id % 256) as u8;

        let fragments = fragment_payload(&encoded, frag_id)?;

        info!(
            "Sending fragmented frame to {} ({} fragments)",
            dest,
            fragments.len()
        );

        for fragment in fragments {
            let mut frag_frame = frame.clone();
            crate::udp::reassembly::add_fragment_headers(&mut frag_frame, &fragment);
            frag_frame.flags.insert(Flags::FRAG);

            let frag_encoded = encode_frame(&frag_frame)?;
            self.socket.send_to(&frag_encoded, dest).await?;

            debug!(
                "Sent fragment {}/{} to {}",
                fragment.frag_index + 1,
                fragment.frag_total,
                dest
            );
        }

        Ok(())
    }

    /// Wait for an ACK for a specific message ID
    async fn wait_for_ack(&mut self, msg_id: u64, from_addr: SocketAddr) -> Result<(), VstpError> {
        let start_time = Instant::now();

        while start_time.elapsed() < self.config.ack_timeout {
            match timeout(Duration::from_millis(100), self.recv()).await {
                Ok(Ok((frame, addr))) if addr == from_addr => {
                    // Check if this is an ACK for our message
                    if frame.typ == FrameType::Ack {
                        for header in &frame.headers {
                            if header.key == b"msg-id" {
                                if let Ok(ack_msg_id) = std::str::from_utf8(&header.value)
                                    .map_err(|e| {
                                        VstpError::Protocol(format!("Invalid UTF-8: {}", e))
                                    })?
                                    .parse::<u64>()
                                {
                                    if ack_msg_id == msg_id {
                                        return Ok(());
                                    }
                                }
                            }
                        }
                    }
                }
                Ok(Ok((_, _))) => {
                    // Frame from different address, ignore
                    continue;
                }
                Ok(Err(e)) => {
                    return Err(e);
                }
                Err(_) => {
                    // Timeout, continue waiting
                    continue;
                }
            }
        }

        Err(VstpError::Timeout("ACK timeout".to_string()))
    }

    /// Calculate retry delay with exponential backoff
    fn calculate_retry_delay(&self, attempt: usize) -> Duration {
        let delay = self.config.retry_delay.as_millis() as u64 * (2_u64.pow(attempt as u32));
        Duration::from_millis(delay.min(self.config.max_retry_delay.as_millis() as u64))
    }

    /// Send an ACK for a received message
    pub async fn send_ack(&self, msg_id: u64, dest: SocketAddr) -> Result<(), VstpError> {
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

        self.send(ack_frame, dest).await
    }

    /// Get the local address this client is bound to
    pub fn local_addr(&self) -> Result<SocketAddr, VstpError> {
        self.socket.local_addr().map_err(|e| VstpError::Io(e))
    }

    /// Get the number of active reassembly sessions
    pub async fn reassembly_session_count(&self) -> usize {
        self.reassembly.session_count().await
    }
}
