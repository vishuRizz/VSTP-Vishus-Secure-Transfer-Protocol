//! Fragmentation and reassembly for UDP frames

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;
use tracing::{debug, warn};

use crate::types::{Frame, VstpError};

/// Maximum size for a single UDP datagram (recommended MTU)
pub const MAX_DATAGRAM_SIZE: usize = 1200;

/// Maximum number of fragments per frame
pub const MAX_FRAGMENTS: usize = 255;

/// Timeout for reassembly (frames not completed within this time are discarded)
pub const REASSEMBLY_TIMEOUT: Duration = Duration::from_secs(30);

/// Maximum number of concurrent reassembly sessions
pub const MAX_REASSEMBLY_SESSIONS: usize = 1000;

/// A fragment of a larger frame
#[derive(Debug, Clone)]
pub struct Fragment {
    pub frag_id: u8,
    pub frag_index: u8,
    pub frag_total: u8,
    pub data: Vec<u8>,
}

/// A reassembly session for a fragmented frame
#[derive(Debug)]
struct ReassemblySession {
    frag_id: u8,
    total_fragments: u8,
    received_fragments: Vec<Option<Vec<u8>>>,
    created_at: Instant,
    from_addr: SocketAddr,
}

impl ReassemblySession {
    fn new(frag_id: u8, total_fragments: u8, from_addr: SocketAddr) -> Self {
        Self {
            frag_id,
            total_fragments,
            received_fragments: vec![None; total_fragments as usize],
            created_at: Instant::now(),
            from_addr,
        }
    }

    fn add_fragment(&mut self, frag_index: u8, data: Vec<u8>) -> Result<(), VstpError> {
        if frag_index >= self.total_fragments {
            return Err(VstpError::Protocol("Invalid fragment index".to_string()));
        }

        if self.received_fragments[frag_index as usize].is_some() {
            return Err(VstpError::Protocol("Duplicate fragment".to_string()));
        }

        self.received_fragments[frag_index as usize] = Some(data);
        Ok(())
    }

    fn is_complete(&self) -> bool {
        self.received_fragments.iter().all(|f| f.is_some())
    }

    fn assemble(&self) -> Result<Vec<u8>, VstpError> {
        if !self.is_complete() {
            return Err(VstpError::Protocol("Frame not complete".to_string()));
        }

        let mut result = Vec::new();
        for fragment in &self.received_fragments {
            if let Some(data) = fragment {
                result.extend_from_slice(data);
            }
        }
        Ok(result)
    }

    fn is_expired(&self) -> bool {
        self.created_at.elapsed() > REASSEMBLY_TIMEOUT
    }
}

/// Manages reassembly of fragmented UDP frames
#[derive(Debug)]
pub struct ReassemblyManager {
    sessions: Arc<Mutex<HashMap<(SocketAddr, u8), ReassemblySession>>>,
}

impl ReassemblyManager {
    pub fn new() -> Self {
        Self {
            sessions: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Add a fragment to the reassembly manager
    pub async fn add_fragment(
        &self,
        from_addr: SocketAddr,
        fragment: Fragment,
    ) -> Result<Option<Vec<u8>>, VstpError> {
        let key = (from_addr, fragment.frag_id);
        let mut sessions = self.sessions.lock().await;

        // Clean up expired sessions first
        self.cleanup_expired(&mut sessions).await;

        // Check if we have too many sessions
        if sessions.len() >= MAX_REASSEMBLY_SESSIONS {
            return Err(VstpError::Protocol(
                "Too many reassembly sessions".to_string(),
            ));
        }

        let session = sessions.entry(key).or_insert_with(|| {
            ReassemblySession::new(fragment.frag_id, fragment.frag_total, from_addr)
        });

        session.add_fragment(fragment.frag_index, fragment.data)?;

        if session.is_complete() {
            let assembled_data = session.assemble()?;
            sessions.remove(&key);
            debug!(
                "Successfully reassembled fragmented frame from {}",
                from_addr
            );
            Ok(Some(assembled_data))
        } else {
            debug!(
                "Fragment {}/{} received from {}",
                fragment.frag_index + 1,
                fragment.frag_total,
                from_addr
            );
            Ok(None)
        }
    }

    /// Clean up expired reassembly sessions
    async fn cleanup_expired(&self, sessions: &mut HashMap<(SocketAddr, u8), ReassemblySession>) {
        let expired_keys: Vec<_> = sessions
            .iter()
            .filter(|(_, session)| session.is_expired())
            .map(|(key, _)| *key)
            .collect();

        for key in expired_keys {
            if let Some(session) = sessions.remove(&key) {
                warn!(
                    "Expired reassembly session for frag_id {} from {}",
                    session.frag_id, session.from_addr
                );
            }
        }
    }

    /// Get the number of active reassembly sessions
    pub async fn session_count(&self) -> usize {
        let sessions = self.sessions.lock().await;
        sessions.len()
    }
}

impl Default for ReassemblyManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Split a large payload into fragments
pub fn fragment_payload(payload: &[u8], frag_id: u8) -> Result<Vec<Fragment>, VstpError> {
    if payload.len() <= MAX_DATAGRAM_SIZE {
        return Ok(vec![]); // No fragmentation needed
    }

    let total_fragments = ((payload.len() + MAX_DATAGRAM_SIZE - 1) / MAX_DATAGRAM_SIZE) as u8;
    if total_fragments > MAX_FRAGMENTS as u8 {
        return Err(VstpError::Protocol(format!(
            "Payload too large: {} fragments needed (max {})",
            total_fragments, MAX_FRAGMENTS
        )));
    }

    let mut fragments = Vec::new();
    for (i, chunk) in payload.chunks(MAX_DATAGRAM_SIZE).enumerate() {
        fragments.push(Fragment {
            frag_id,
            frag_index: i as u8,
            frag_total: total_fragments,
            data: chunk.to_vec(),
        });
    }

    Ok(fragments)
}

/// Extract fragment information from frame headers
pub fn extract_fragment_info(frame: &Frame) -> Option<Fragment> {
    // Look for fragment headers
    for header in &frame.headers {
        if header.key == b"frag-id" {
            if let Some(frag_index_header) = frame.headers.iter().find(|h| h.key == b"frag-index") {
                if let Some(frag_total_header) =
                    frame.headers.iter().find(|h| h.key == b"frag-total")
                {
                    if let (Ok(frag_id), Ok(frag_index), Ok(frag_total)) = (
                        std::str::from_utf8(&header.value)
                            .unwrap_or("0")
                            .parse::<u8>(),
                        std::str::from_utf8(&frag_index_header.value)
                            .unwrap_or("0")
                            .parse::<u8>(),
                        std::str::from_utf8(&frag_total_header.value)
                            .unwrap_or("1")
                            .parse::<u8>(),
                    ) {
                        return Some(Fragment {
                            frag_id,
                            frag_index,
                            frag_total,
                            data: frame.payload.clone(),
                        });
                    }
                }
            }
        }
    }
    None
}

/// Add fragment headers to a frame
pub fn add_fragment_headers(frame: &mut Frame, fragment: &Fragment) {
    frame.headers.push(crate::types::Header {
        key: b"frag-id".to_vec(),
        value: fragment.frag_id.to_string().into_bytes(),
    });
    frame.headers.push(crate::types::Header {
        key: b"frag-index".to_vec(),
        value: fragment.frag_index.to_string().into_bytes(),
    });
    frame.headers.push(crate::types::Header {
        key: b"frag-total".to_vec(),
        value: fragment.frag_total.to_string().into_bytes(),
    });
}
