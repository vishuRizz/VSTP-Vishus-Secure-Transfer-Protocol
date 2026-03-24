use crate::{Flags, Frame, FrameType, VstpError};
use serde::{de::DeserializeOwned, Serialize};
use std::collections::HashMap;
use std::time::Instant;
use std::{net::SocketAddr, sync::Arc, time::Duration};
use tokio::sync::{mpsc, Mutex};

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);
const DEFAULT_PREFERRED_MARGIN_MS: f64 = 5.0;
const DEFAULT_PEER_PREF_TTL: Duration = Duration::from_secs(120);

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum TransportKind {
    Tcp,
    Udp,
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct AutoFaultInjection {
    pub tcp_delay_ms: u64,
    pub udp_delay_ms: u64,
    pub tcp_fail_every_n: u64,
    pub udp_fail_every_n: u64,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct AutoRuntimeStatus {
    pub active_transport: TransportKind,
    pub tcp_available: bool,
    pub udp_available: bool,
    pub tcp_score_ms: f64,
    pub udp_score_ms: f64,
    pub tcp_success_count: u64,
    pub udp_success_count: u64,
    pub tcp_failure_count: u64,
    pub udp_failure_count: u64,
    pub tcp_timeout_count: u64,
    pub udp_timeout_count: u64,
    pub tcp_ema_rtt_ms: Option<f64>,
    pub udp_ema_rtt_ms: Option<f64>,
    pub fault_injection: AutoFaultInjection,
}

#[derive(Debug, Clone)]
pub struct AutoSwitchConfig {
    pub probe_attempts: usize,
    pub probe_timeout: Duration,
    pub switch_cooldown: Duration,
    pub min_dwell_time: Duration,
    pub consecutive_failures_threshold: u32,
    pub min_score_margin_ms: f64,
    pub ack_timeout_penalty_ms: f64,
    pub io_error_penalty_ms: f64,
    pub peer_preference_ttl: Duration,
}

impl Default for AutoSwitchConfig {
    fn default() -> Self {
        Self {
            probe_attempts: 2,
            probe_timeout: Duration::from_millis(600),
            switch_cooldown: Duration::from_secs(3),
            min_dwell_time: Duration::from_secs(2),
            consecutive_failures_threshold: 3,
            min_score_margin_ms: DEFAULT_PREFERRED_MARGIN_MS,
            ack_timeout_penalty_ms: 25.0,
            io_error_penalty_ms: 35.0,
            peer_preference_ttl: DEFAULT_PEER_PREF_TTL,
        }
    }
}

#[derive(Debug, Clone, Default)]
struct TransportStats {
    success_count: u64,
    failure_count: u64,
    timeout_count: u64,
    retry_exhausted_count: u64,
    consecutive_failures: u32,
    ema_rtt_ms: Option<f64>,
}

impl TransportStats {
    fn score_ms(&self, cfg: &AutoSwitchConfig) -> f64 {
        let base = self.ema_rtt_ms.unwrap_or(50.0);
        base + (self.timeout_count as f64 * cfg.ack_timeout_penalty_ms)
            + (self.retry_exhausted_count as f64 * cfg.ack_timeout_penalty_ms)
            + (self.failure_count as f64 * cfg.io_error_penalty_ms)
    }

    fn mark_success(&mut self, elapsed: Duration) {
        self.success_count += 1;
        self.consecutive_failures = 0;
        let sample = elapsed.as_secs_f64() * 1000.0;
        self.ema_rtt_ms = Some(match self.ema_rtt_ms {
            Some(prev) => (prev * 0.8) + (sample * 0.2),
            None => sample,
        });
    }

    fn mark_failure(&mut self, err: &VstpError) {
        self.failure_count += 1;
        self.consecutive_failures += 1;
        if matches!(err, VstpError::Timeout) {
            self.timeout_count += 1;
            self.retry_exhausted_count += 1;
        }
    }
}

#[derive(Debug, Clone)]
struct AutoClientState {
    active: TransportKind,
    last_switch_at: Instant,
    active_since: Instant,
    tcp_stats: TransportStats,
    udp_stats: TransportStats,
}

impl AutoClientState {
    fn new(initial: TransportKind) -> Self {
        let now = Instant::now();
        Self {
            active: initial,
            last_switch_at: now,
            active_since: now,
            tcp_stats: TransportStats::default(),
            udp_stats: TransportStats::default(),
        }
    }

    fn stats_mut(&mut self, transport: TransportKind) -> &mut TransportStats {
        match transport {
            TransportKind::Tcp => &mut self.tcp_stats,
            TransportKind::Udp => &mut self.udp_stats,
        }
    }

    fn stats(&self, transport: TransportKind) -> &TransportStats {
        match transport {
            TransportKind::Tcp => &self.tcp_stats,
            TransportKind::Udp => &self.udp_stats,
        }
    }
}

/// A simplified client that handles both TCP and UDP connections
#[derive(Clone)]
pub struct VstpClient {
    inner: Arc<Mutex<ClientType>>,
    server_addr: SocketAddr,
    timeout: Duration,
}

enum ClientType {
    Tcp(crate::tcp::VstpTcpClient),
    Udp(crate::udp::VstpUdpClient),
    Auto(AutoClientInner),
}

struct AutoClientInner {
    tcp: Option<crate::tcp::VstpTcpClient>,
    udp: Option<crate::udp::VstpUdpClient>,
    state: AutoClientState,
    cfg: AutoSwitchConfig,
    fault: AutoFaultInjection,
    op_counter: u64,
}

impl VstpClient {
    /// Connect to a TCP server with automatic TLS
    pub async fn connect_tcp(addr: impl Into<String>) -> Result<Self, VstpError> {
        let addr_str = addr.into();
        let server_addr = addr_str
            .parse()
            .map_err(|e| VstpError::Protocol(format!("Invalid address: {}", e)))?;
        let client = crate::tcp::VstpTcpClient::connect(&addr_str).await?;

        Ok(Self {
            inner: Arc::new(Mutex::new(ClientType::Tcp(client))),
            server_addr,
            timeout: DEFAULT_TIMEOUT,
        })
    }

    /// Create a UDP client bound to any port
    pub async fn connect_udp(server_addr: impl Into<String>) -> Result<Self, VstpError> {
        let addr_str = server_addr.into();
        let server_addr = addr_str
            .parse()
            .map_err(|e| VstpError::Protocol(format!("Invalid address: {}", e)))?;
        let client = crate::udp::VstpUdpClient::bind("0.0.0.0:0").await?;

        Ok(Self {
            inner: Arc::new(Mutex::new(ClientType::Udp(client))),
            server_addr,
            timeout: DEFAULT_TIMEOUT,
        })
    }

    /// Connect with automatic transport probing and adaptive switching.
    pub async fn connect_auto(server_addr: impl Into<String>) -> Result<Self, VstpError> {
        Self::connect_auto_with_config(server_addr, AutoSwitchConfig::default()).await
    }

    pub async fn connect_auto_with_config(
        server_addr: impl Into<String>,
        cfg: AutoSwitchConfig,
    ) -> Result<Self, VstpError> {
        let addr_str = server_addr.into();
        let parsed_addr = addr_str
            .parse()
            .map_err(|e| VstpError::Protocol(format!("Invalid address: {}", e)))?;

        let probe_count = cfg.probe_attempts.max(1);
        let mut tcp_best_ms: Option<f64> = None;
        let mut udp_best_ms: Option<f64> = None;
        let mut tcp_client_opt = None;
        let mut udp_client_opt = None;
        let mut last_tcp_err = None;
        let mut last_udp_err = None;

        for _ in 0..probe_count {
            if udp_client_opt.is_none() {
                match crate::udp::VstpUdpClient::bind("0.0.0.0:0").await {
                    Ok(client) => udp_client_opt = Some(client),
                    Err(e) => last_udp_err = Some(e),
                }
            }
            let tcp_start = Instant::now();
            let udp_start = Instant::now();
            let probe = Frame::new(FrameType::Ping)
                .with_flag(Flags::REQ_ACK)
                .with_header("x-auto-probe", "1");
            let tcp_probe = tokio::time::timeout(cfg.probe_timeout, crate::tcp::VstpTcpClient::connect(&addr_str));
            let udp_probe = async {
                if let Some(udp_client) = udp_client_opt.as_mut() {
                    match tokio::time::timeout(cfg.probe_timeout, udp_client.send_with_ack(probe, parsed_addr)).await {
                        Ok(Ok(())) => Ok(()),
                        Ok(Err(e)) => Err(e),
                        Err(_) => Err(VstpError::Timeout),
                    }
                } else {
                    Err(VstpError::Protocol("UDP probe unavailable".to_string()))
                }
            };

            let (tcp_result, udp_result) = tokio::join!(tcp_probe, udp_probe);
            match tcp_result {
                Ok(Ok(client)) => {
                    let ms = tcp_start.elapsed().as_secs_f64() * 1000.0;
                    tcp_best_ms = Some(tcp_best_ms.map_or(ms, |prev| prev.min(ms)));
                    if tcp_client_opt.is_none() {
                        tcp_client_opt = Some(client);
                    }
                }
                Ok(Err(e)) => last_tcp_err = Some(e),
                Err(_) => last_tcp_err = Some(VstpError::Timeout),
            }

            match udp_result {
                Ok(()) => {
                    let ms = udp_start.elapsed().as_secs_f64() * 1000.0;
                    udp_best_ms = Some(udp_best_ms.map_or(ms, |prev| prev.min(ms)));
                }
                Err(e) => last_udp_err = Some(e),
            }
        }

        if tcp_client_opt.is_none() && udp_client_opt.is_none() {
            return Err(VstpError::Protocol(format!(
                "Auto probe failed (tcp: {}, udp: {})",
                last_tcp_err
                    .map(|e| e.to_string())
                    .unwrap_or_else(|| "unavailable".to_string()),
                last_udp_err
                    .map(|e| e.to_string())
                    .unwrap_or_else(|| "unavailable".to_string())
            )));
        }

        let tcp_score = tcp_best_ms.unwrap_or(1000.0);
        let udp_score = udp_best_ms.unwrap_or(1000.0);
        let initial = match (tcp_client_opt.is_some(), udp_client_opt.is_some()) {
            (true, true) => {
                if udp_score + cfg.min_score_margin_ms < tcp_score {
                    TransportKind::Udp
                } else {
                    TransportKind::Tcp
                }
            }
            (true, false) => TransportKind::Tcp,
            (false, true) => TransportKind::Udp,
            (false, false) => unreachable!(),
        };

        Ok(Self {
            inner: Arc::new(Mutex::new(ClientType::Auto(AutoClientInner {
                tcp: tcp_client_opt,
                udp: udp_client_opt,
                state: AutoClientState::new(initial),
                cfg,
                fault: AutoFaultInjection::default(),
                op_counter: 0,
            }))),
            server_addr: parsed_addr,
            timeout: DEFAULT_TIMEOUT,
        })
    }

    /// Set operation timeout
    pub fn set_timeout(&mut self, timeout: Duration) {
        self.timeout = timeout;
    }

    /// Update runtime fault injection values for auto mode.
    pub async fn set_auto_fault_injection(
        &self,
        fault: AutoFaultInjection,
    ) -> Result<(), VstpError> {
        let mut inner = self.inner.lock().await;
        match &mut *inner {
            ClientType::Auto(auto) => {
                auto.fault = fault;
                Ok(())
            }
            _ => Err(VstpError::Protocol(
                "Fault injection is available only in auto mode".to_string(),
            )),
        }
    }

    /// Inspect current auto-transport runtime status.
    pub async fn auto_status(&self) -> Result<AutoRuntimeStatus, VstpError> {
        let inner = self.inner.lock().await;
        match &*inner {
            ClientType::Auto(auto) => Ok(AutoRuntimeStatus {
                active_transport: auto.state.active,
                tcp_available: auto.tcp.is_some(),
                udp_available: auto.udp.is_some(),
                tcp_score_ms: auto.state.tcp_stats.score_ms(&auto.cfg),
                udp_score_ms: auto.state.udp_stats.score_ms(&auto.cfg),
                tcp_success_count: auto.state.tcp_stats.success_count,
                udp_success_count: auto.state.udp_stats.success_count,
                tcp_failure_count: auto.state.tcp_stats.failure_count,
                udp_failure_count: auto.state.udp_stats.failure_count,
                tcp_timeout_count: auto.state.tcp_stats.timeout_count,
                udp_timeout_count: auto.state.udp_stats.timeout_count,
                tcp_ema_rtt_ms: auto.state.tcp_stats.ema_rtt_ms,
                udp_ema_rtt_ms: auto.state.udp_stats.ema_rtt_ms,
                fault_injection: auto.fault.clone(),
            }),
            _ => Err(VstpError::Protocol(
                "Status is available only in auto mode".to_string(),
            )),
        }
    }

    /// Send any serializable data to the server
    pub async fn send<T: Serialize>(&self, data: T) -> Result<(), VstpError> {
        let payload = serde_json::to_vec(&data)
            .map_err(|e| VstpError::Protocol(format!("Serialization error: {}", e)))?;
        let frame = Frame::new(FrameType::Data)
            .with_header("content-type", "application/json")
            .with_payload(payload);

        let mut inner = self.inner.lock().await;
        match &mut *inner {
            ClientType::Tcp(client) => tokio::time::timeout(self.timeout, client.send(frame))
                .await
                .map_err(|_| VstpError::Timeout)?
                .map_err(|e| VstpError::Protocol(format!("Send error: {}", e)))?,
            ClientType::Udp(client) => {
                tokio::time::timeout(self.timeout, client.send(frame, self.server_addr))
                    .await
                    .map_err(|_| VstpError::Timeout)?
                    .map_err(|e| VstpError::Protocol(format!("Send error: {}", e)))?
            }
            ClientType::Auto(auto) => {
                self.auto_send_with_fallback(auto, frame, false).await?;
            }
        }
        Ok(())
    }

    /// Send a raw frame directly
    pub async fn send_raw(&self, frame: Frame) -> Result<(), VstpError> {
        let mut inner = self.inner.lock().await;
        match &mut *inner {
            ClientType::Tcp(client) => tokio::time::timeout(self.timeout, client.send(frame))
                .await
                .map_err(|_| VstpError::Timeout)?
                .map_err(|e| VstpError::Protocol(format!("Send error: {}", e)))?,
            ClientType::Udp(client) => {
                tokio::time::timeout(self.timeout, client.send(frame, self.server_addr))
                    .await
                    .map_err(|_| VstpError::Timeout)?
                    .map_err(|e| VstpError::Protocol(format!("Send error: {}", e)))?
            }
            ClientType::Auto(auto) => {
                self.auto_send_with_fallback(auto, frame, false).await?;
            }
        }
        Ok(())
    }

    /// Receive data and automatically deserialize it
    pub async fn receive<T: DeserializeOwned>(&self) -> Result<T, VstpError> {
        let mut inner = self.inner.lock().await;
        let frame = match &mut *inner {
            ClientType::Tcp(client) => tokio::time::timeout(self.timeout, client.recv())
                .await
                .map_err(|_| VstpError::Timeout)?
                .map_err(|e| VstpError::Protocol(format!("Receive error: {}", e)))?
                .ok_or_else(|| VstpError::Protocol("Connection closed".to_string()))?,
            ClientType::Udp(client) => {
                let (frame, _) = tokio::time::timeout(self.timeout, client.recv())
                    .await
                    .map_err(|_| VstpError::Timeout)?
                    .map_err(|e| VstpError::Protocol(format!("Receive error: {}", e)))?;
                frame
            }
            ClientType::Auto(auto) => {
                self.auto_recv_with_fallback(auto).await?
            }
        };

        serde_json::from_slice(frame.payload())
            .map_err(|e| VstpError::Protocol(format!("Deserialization error: {}", e)))
    }

    /// Send data and wait for acknowledgment
    pub async fn send_with_ack<T: Serialize>(&self, data: T) -> Result<(), VstpError> {
        let payload = serde_json::to_vec(&data)
            .map_err(|e| VstpError::Protocol(format!("Serialization error: {}", e)))?;
        let frame = Frame::new(FrameType::Data)
            .with_header("content-type", "application/json")
            .with_flag(Flags::REQ_ACK)
            .with_payload(payload);

        let mut inner = self.inner.lock().await;
        match &mut *inner {
            ClientType::Tcp(client) => tokio::time::timeout(self.timeout, async {
                client.send(frame).await?;
                let ack = client
                    .recv()
                    .await?
                    .ok_or_else(|| VstpError::Protocol("Connection closed".to_string()))?;
                if ack.frame_type() != FrameType::Ack {
                    return Err(VstpError::Protocol("Expected ACK frame".to_string()));
                }
                Ok(())
            })
            .await
            .map_err(|_| VstpError::Timeout)??,
            ClientType::Udp(client) => {
                tokio::time::timeout(self.timeout, client.send_with_ack(frame, self.server_addr))
                    .await
                    .map_err(|_| VstpError::Timeout)??
            }
            ClientType::Auto(auto) => {
                self.auto_send_with_fallback(auto, frame, true).await?;
            }
        }
        Ok(())
    }

    fn maybe_switch_transport(auto: &mut AutoClientInner) {
        let now = Instant::now();
        if now.duration_since(auto.state.last_switch_at) < auto.cfg.switch_cooldown
            || now.duration_since(auto.state.active_since) < auto.cfg.min_dwell_time
        {
            return;
        }

        let active = auto.state.active;
        let alt = if active == TransportKind::Tcp {
            TransportKind::Udp
        } else {
            TransportKind::Tcp
        };
        let alt_available = match alt {
            TransportKind::Tcp => auto.tcp.is_some(),
            TransportKind::Udp => auto.udp.is_some(),
        };
        if !alt_available {
            return;
        }

        let active_score = auto.state.stats(active).score_ms(&auto.cfg);
        let alt_score = auto.state.stats(alt).score_ms(&auto.cfg);
        let active_failures = auto.state.stats(active).consecutive_failures;
        let should_switch = active_failures >= auto.cfg.consecutive_failures_threshold
            || (active_score - alt_score) > auto.cfg.min_score_margin_ms;
        if should_switch {
            auto.state.active = alt;
            auto.state.last_switch_at = now;
            auto.state.active_since = now;
        }
    }

    async fn auto_send_with_fallback(
        &self,
        auto: &mut AutoClientInner,
        frame: Frame,
        require_ack: bool,
    ) -> Result<(), VstpError> {
        let active = auto.state.active;
        let first_try = self
            .send_on_transport(active, auto, frame.clone(), require_ack)
            .await;
        match first_try {
            Ok(elapsed) => {
                auto.state.stats_mut(active).mark_success(elapsed);
                Self::maybe_switch_transport(auto);
                Ok(())
            }
            Err(err) => {
                auto.state.stats_mut(active).mark_failure(&err);
                let fallback = if active == TransportKind::Tcp {
                    TransportKind::Udp
                } else {
                    TransportKind::Tcp
                };
                let fallback_available = match fallback {
                    TransportKind::Tcp => auto.tcp.is_some(),
                    TransportKind::Udp => auto.udp.is_some(),
                };
                if !fallback_available {
                    Self::maybe_switch_transport(auto);
                    return Err(err);
                }
                let second_try = self
                    .send_on_transport(fallback, auto, frame, require_ack)
                    .await;
                match second_try {
                    Ok(elapsed) => {
                        auto.state.stats_mut(fallback).mark_success(elapsed);
                        auto.state.active = fallback;
                        auto.state.last_switch_at = Instant::now();
                        auto.state.active_since = Instant::now();
                        Ok(())
                    }
                    Err(second_err) => {
                        auto.state.stats_mut(fallback).mark_failure(&second_err);
                        Self::maybe_switch_transport(auto);
                        Err(second_err)
                    }
                }
            }
        }
    }

    async fn auto_recv_with_fallback(&self, auto: &mut AutoClientInner) -> Result<Frame, VstpError> {
        let active = auto.state.active;
        let first = self.recv_on_transport(active, auto).await;
        match first {
            Ok((frame, elapsed)) => {
                auto.state.stats_mut(active).mark_success(elapsed);
                Self::maybe_switch_transport(auto);
                Ok(frame)
            }
            Err(err) => {
                auto.state.stats_mut(active).mark_failure(&err);
                let fallback = if active == TransportKind::Tcp {
                    TransportKind::Udp
                } else {
                    TransportKind::Tcp
                };
                let fallback_available = match fallback {
                    TransportKind::Tcp => auto.tcp.is_some(),
                    TransportKind::Udp => auto.udp.is_some(),
                };
                if !fallback_available {
                    Self::maybe_switch_transport(auto);
                    return Err(err);
                }
                let second = self.recv_on_transport(fallback, auto).await;
                match second {
                    Ok((frame, elapsed)) => {
                        auto.state.stats_mut(fallback).mark_success(elapsed);
                        auto.state.active = fallback;
                        auto.state.last_switch_at = Instant::now();
                        auto.state.active_since = Instant::now();
                        Ok(frame)
                    }
                    Err(second_err) => {
                        auto.state.stats_mut(fallback).mark_failure(&second_err);
                        Self::maybe_switch_transport(auto);
                        Err(second_err)
                    }
                }
            }
        }
    }

    async fn send_on_transport(
        &self,
        transport: TransportKind,
        auto: &mut AutoClientInner,
        frame: Frame,
        require_ack: bool,
    ) -> Result<Duration, VstpError> {
        auto.op_counter = auto.op_counter.saturating_add(1);
        self.apply_fault(transport, auto).await?;
        let start = Instant::now();
        match transport {
            TransportKind::Tcp => {
                let tcp = auto
                    .tcp
                    .as_mut()
                    .ok_or_else(|| VstpError::Protocol("TCP transport unavailable".to_string()))?;
                if require_ack {
                    tokio::time::timeout(self.timeout, async {
                        tcp.send(frame).await?;
                        let ack = tcp
                            .recv()
                            .await?
                            .ok_or_else(|| VstpError::Protocol("Connection closed".to_string()))?;
                        if ack.frame_type() != FrameType::Ack {
                            return Err(VstpError::Protocol("Expected ACK frame".to_string()));
                        }
                        Ok::<(), VstpError>(())
                    })
                    .await
                    .map_err(|_| VstpError::Timeout)??;
                } else {
                    tokio::time::timeout(self.timeout, tcp.send(frame))
                        .await
                        .map_err(|_| VstpError::Timeout)??;
                }
            }
            TransportKind::Udp => {
                let udp = auto
                    .udp
                    .as_mut()
                    .ok_or_else(|| VstpError::Protocol("UDP transport unavailable".to_string()))?;
                if require_ack {
                    tokio::time::timeout(self.timeout, udp.send_with_ack(frame, self.server_addr))
                        .await
                        .map_err(|_| VstpError::Timeout)??;
                } else {
                    tokio::time::timeout(self.timeout, udp.send(frame, self.server_addr))
                        .await
                        .map_err(|_| VstpError::Timeout)??;
                }
            }
        }
        Ok(start.elapsed())
    }

    async fn recv_on_transport(
        &self,
        transport: TransportKind,
        auto: &mut AutoClientInner,
    ) -> Result<(Frame, Duration), VstpError> {
        auto.op_counter = auto.op_counter.saturating_add(1);
        self.apply_fault(transport, auto).await?;
        let start = Instant::now();
        let frame = match transport {
            TransportKind::Tcp => {
                let tcp = auto
                    .tcp
                    .as_mut()
                    .ok_or_else(|| VstpError::Protocol("TCP transport unavailable".to_string()))?;
                tokio::time::timeout(self.timeout, tcp.recv())
                .await
                .map_err(|_| VstpError::Timeout)??
                .ok_or_else(|| VstpError::Protocol("Connection closed".to_string()))?
            }
            TransportKind::Udp => {
                let udp = auto
                    .udp
                    .as_mut()
                    .ok_or_else(|| VstpError::Protocol("UDP transport unavailable".to_string()))?;
                let (frame, _) = tokio::time::timeout(self.timeout, udp.recv())
                    .await
                    .map_err(|_| VstpError::Timeout)??;
                frame
            }
        };
        Ok((frame, start.elapsed()))
    }

    async fn apply_fault(
        &self,
        transport: TransportKind,
        auto: &AutoClientInner,
    ) -> Result<(), VstpError> {
        let (delay_ms, fail_every_n) = match transport {
            TransportKind::Tcp => (auto.fault.tcp_delay_ms, auto.fault.tcp_fail_every_n),
            TransportKind::Udp => (auto.fault.udp_delay_ms, auto.fault.udp_fail_every_n),
        };
        if delay_ms > 0 {
            tokio::time::sleep(Duration::from_millis(delay_ms)).await;
        }
        if fail_every_n > 0 && auto.op_counter % fail_every_n == 0 {
            return Err(VstpError::Timeout);
        }
        Ok(())
    }
}

/// A simplified server that handles connections and message routing
pub struct VstpServer {
    inner: ServerType,
    message_tx: mpsc::Sender<ServerMessage>,
    message_rx: mpsc::Receiver<ServerMessage>,
    timeout: Duration,
}

enum ServerType {
    Tcp(crate::tcp::VstpTcpServer),
    Udp(crate::udp::VstpUdpServer),
    Auto(AutoServerInner),
}

struct AutoServerInner {
    tcp: Arc<crate::tcp::VstpTcpServer>,
    udp: Arc<crate::udp::VstpUdpServer>,
    cfg: AutoSwitchConfig,
    peer_preference: Arc<Mutex<HashMap<SocketAddr, PeerPreference>>>,
}

#[derive(Clone, Copy)]
struct PeerPreference {
    transport: TransportKind,
    last_seen: Instant,
}

struct ServerMessage {
    data: Vec<u8>,
    _client_addr: SocketAddr,
    response_tx: mpsc::Sender<Vec<u8>>,
}

impl VstpServer {
    /// Create a new TCP server with automatic TLS
    pub async fn bind_tcp(addr: impl Into<String>) -> Result<Self, VstpError> {
        let addr_str = addr.into();
        let server = crate::tcp::VstpTcpServer::bind(&addr_str).await?;
        let (tx, rx) = mpsc::channel(100);
        Ok(Self {
            inner: ServerType::Tcp(server),
            message_tx: tx,
            message_rx: rx,
            timeout: DEFAULT_TIMEOUT,
        })
    }

    /// Create a new UDP server
    pub async fn bind_udp(addr: impl Into<String>) -> Result<Self, VstpError> {
        let addr_str = addr.into();
        let server = crate::udp::VstpUdpServer::bind(&addr_str).await?;
        let (tx, rx) = mpsc::channel(100);
        Ok(Self {
            inner: ServerType::Udp(server),
            message_tx: tx,
            message_rx: rx,
            timeout: DEFAULT_TIMEOUT,
        })
    }

    pub async fn bind_auto(addr: impl Into<String>) -> Result<Self, VstpError> {
        Self::bind_auto_with_config(addr, AutoSwitchConfig::default()).await
    }

    pub async fn bind_auto_with_config(
        addr: impl Into<String>,
        cfg: AutoSwitchConfig,
    ) -> Result<Self, VstpError> {
        let addr_str = addr.into();
        let tcp = crate::tcp::VstpTcpServer::bind(&addr_str).await?;
        let udp = crate::udp::VstpUdpServer::bind(&addr_str).await?;
        let (tx, rx) = mpsc::channel(100);
        Ok(Self {
            inner: ServerType::Auto(AutoServerInner {
                tcp: Arc::new(tcp),
                udp: Arc::new(udp),
                cfg,
                peer_preference: Arc::new(Mutex::new(HashMap::new())),
            }),
            message_tx: tx,
            message_rx: rx,
            timeout: DEFAULT_TIMEOUT,
        })
    }

    /// Set operation timeout
    pub fn set_timeout(&mut self, timeout: Duration) {
        self.timeout = timeout;
    }

    /// Start the server and handle incoming messages with the provided handler
    pub async fn serve<F, Fut, T, R>(mut self, handler: F) -> Result<(), VstpError>
    where
        F: Fn(T) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = Result<R, VstpError>> + Send,
        T: DeserializeOwned + Send + 'static,
        R: Serialize + Send + 'static,
    {
        let handler = Arc::new(handler);

        match self.inner {
            ServerType::Tcp(server) => {
                let tx = self.message_tx.clone();
                let timeout = self.timeout;

                tokio::spawn(async move {
                    loop {
                        let mut client = server.accept().await?;
                        let tx = tx.clone();

                        tokio::spawn(async move {
                            while let Ok(Some(frame)) = client.recv().await {
                                if frame.get_header("x-auto-probe") == Some("1") {
                                    continue;
                                }
                                let (response_tx, mut response_rx) = mpsc::channel(1);

                                // Try to deserialize and handle the message
                                match serde_json::from_slice::<T>(&frame.payload()) {
                                    Ok(_data) => {
                                        if let Err(_) = tokio::time::timeout(
                                            timeout,
                                            tx.send(ServerMessage {
                                                data: frame.payload().to_vec(),
                                                _client_addr: client.peer_addr(),
                                                response_tx,
                                            }),
                                        )
                                        .await
                                        {
                                            break;
                                        }

                                        if let Some(response) = response_rx.recv().await {
                                            let response_frame =
                                                Frame::new(FrameType::Data).with_payload(response);
                                            if let Err(_) = client.send(response_frame).await {
                                                break;
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        // Send error response for invalid data
                                        let error_frame = Frame::new(FrameType::Data).with_payload(
                                            format!("Invalid data: {}", e).into_bytes(),
                                        );
                                        let _ = client.send(error_frame).await;
                                    }
                                }
                            }
                            Ok::<_, VstpError>(())
                        });
                    }
                    #[allow(unreachable_code)]
                    Ok::<_, VstpError>(())
                });
            }
            ServerType::Udp(server) => {
                let tx = self.message_tx.clone();
                let timeout = self.timeout;

                tokio::spawn(async move {
                    while let Ok((frame, addr)) = server.recv().await {
                        if frame.get_header("x-auto-probe") == Some("1") {
                            continue;
                        }
                        let (response_tx, mut response_rx) = mpsc::channel(1);

                        // Try to deserialize and handle the message
                        match serde_json::from_slice::<T>(&frame.payload()) {
                            Ok(_data) => {
                                if let Err(_) = tokio::time::timeout(
                                    timeout,
                                    tx.send(ServerMessage {
                                        data: frame.payload().to_vec(),
                                        _client_addr: addr,
                                        response_tx,
                                    }),
                                )
                                .await
                                {
                                    break;
                                }

                                if let Some(response) = response_rx.recv().await {
                                    let response_frame =
                                        Frame::new(FrameType::Data).with_payload(response);
                                    let _ = server.send(response_frame, addr).await;
                                }
                            }
                            Err(e) => {
                                // Send error response for invalid data
                                let error_frame = Frame::new(FrameType::Data)
                                    .with_payload(format!("Invalid data: {}", e).into_bytes());
                                let _ = server.send(error_frame, addr).await;
                            }
                        }
                    }
                });
            }
            ServerType::Auto(auto) => {
                let tx_tcp = self.message_tx.clone();
                let tx_udp = self.message_tx.clone();
                let timeout = self.timeout;
                let pref_tcp = auto.peer_preference.clone();
                let pref_udp = auto.peer_preference.clone();
                let ttl = auto.cfg.peer_preference_ttl;
                let tcp_server = auto.tcp.clone();
                let udp_server = auto.udp.clone();

                tokio::spawn(async move {
                    loop {
                        let mut client = tcp_server.accept().await?;
                        let tx = tx_tcp.clone();
                        let pref = pref_tcp.clone();
                        tokio::spawn(async move {
                            while let Ok(Some(frame)) = client.recv().await {
                                if frame.get_header("x-auto-probe") == Some("1") {
                                    continue;
                                }
                                {
                                    let mut guard = pref.lock().await;
                                    guard.insert(
                                        client.peer_addr(),
                                        PeerPreference {
                                            transport: TransportKind::Tcp,
                                            last_seen: Instant::now(),
                                        },
                                    );
                                }
                                let (response_tx, mut response_rx) = mpsc::channel(1);
                                if tokio::time::timeout(
                                    timeout,
                                    tx.send(ServerMessage {
                                        data: frame.payload().to_vec(),
                                        _client_addr: client.peer_addr(),
                                        response_tx,
                                    }),
                                )
                                .await
                                .is_err()
                                {
                                    break;
                                }

                                if let Some(response) = response_rx.recv().await {
                                    let response_frame = Frame::new(FrameType::Data).with_payload(response);
                                    if client.send(response_frame).await.is_err() {
                                        break;
                                    }
                                }

                                {
                                    let mut guard = pref.lock().await;
                                    guard.retain(|_, v| v.last_seen.elapsed() <= ttl);
                                }
                            }
                        });
                    }
                    #[allow(unreachable_code)]
                    Ok::<_, VstpError>(())
                });

                tokio::spawn(async move {
                    while let Ok((frame, addr)) = udp_server.recv().await {
                        if frame.get_header("x-auto-probe") == Some("1") {
                            continue;
                        }
                        {
                            let mut guard = pref_udp.lock().await;
                            guard.insert(
                                addr,
                                PeerPreference {
                                    transport: TransportKind::Udp,
                                    last_seen: Instant::now(),
                                },
                            );
                        }

                        let (response_tx, mut response_rx) = mpsc::channel(1);
                        if tokio::time::timeout(
                            timeout,
                            tx_udp.send(ServerMessage {
                                data: frame.payload().to_vec(),
                                _client_addr: addr,
                                response_tx,
                            }),
                        )
                        .await
                        .is_err()
                        {
                            break;
                        }

                        if let Some(response) = response_rx.recv().await {
                            let response_frame = Frame::new(FrameType::Data).with_payload(response);
                            let preferred = {
                                let guard = pref_udp.lock().await;
                                guard.get(&addr).copied()
                            };
                            let should_send_udp = preferred
                                .map(|p| p.transport == TransportKind::Udp)
                                .unwrap_or(true);
                            if should_send_udp {
                                let _ = udp_server.send(response_frame, addr).await;
                            }
                        }
                    }
                });
            }
        }

        while let Some(msg) = self.message_rx.recv().await {
            let handler = handler.clone();
            tokio::spawn(async move {
                match serde_json::from_slice::<T>(&msg.data) {
                    Ok(data) => match handler(data).await {
                        Ok(response) => {
                            if let Ok(response_data) = serde_json::to_vec(&response) {
                                let _ = msg.response_tx.send(response_data).await;
                            }
                        }
                        Err(_) => (),
                    },
                    Err(_) => (),
                }
            });
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::{Deserialize, Serialize};
    use tokio;

    #[derive(Debug, Serialize, Deserialize, PartialEq, Clone)]
    struct TestMessage {
        content: String,
    }

    #[tokio::test]
    async fn test_tcp_echo() -> Result<(), VstpError> {
        let server = VstpServer::bind_tcp("127.0.0.1:8081").await?;
        tokio::spawn(async move {
            server
                .serve(|msg: TestMessage| async move { Ok(msg) })
                .await
        });

        tokio::time::sleep(Duration::from_millis(100)).await;

        let client = VstpClient::connect_tcp("127.0.0.1:8081").await?;

        let msg = TestMessage {
            content: "Hello VSTP!".to_string(),
        };
        client.send(msg.clone()).await?;
        let response: TestMessage = client.receive().await?;

        assert_eq!(msg, response);
        Ok(())
    }

    #[tokio::test]
    async fn test_udp_echo() -> Result<(), VstpError> {
        let server = VstpServer::bind_udp("127.0.0.1:8082").await?;
        tokio::spawn(async move {
            server
                .serve(|msg: TestMessage| async move { Ok(msg) })
                .await
        });

        tokio::time::sleep(Duration::from_millis(100)).await;

        let client = VstpClient::connect_udp("127.0.0.1:8082").await?;

        let msg = TestMessage {
            content: "Hello UDP VSTP!".to_string(),
        };
        client.send(msg.clone()).await?;
        let response: TestMessage = client.receive().await?;

        assert_eq!(msg, response);
        Ok(())
    }

    #[tokio::test]
    async fn test_tcp_timeout() -> Result<(), VstpError> {
        let server = VstpServer::bind_tcp("127.0.0.1:8083").await?;
        tokio::spawn(async move {
            server
                .serve(|msg: TestMessage| async move {
                    tokio::time::sleep(Duration::from_secs(10)).await;
                    Ok(msg)
                })
                .await
        });

        tokio::time::sleep(Duration::from_millis(100)).await;

        let mut client = VstpClient::connect_tcp("127.0.0.1:8083").await?;
        client.set_timeout(Duration::from_millis(100));

        let msg = TestMessage {
            content: "Should timeout".to_string(),
        };
        client.send(msg).await?;

        match client.receive::<TestMessage>().await {
            Err(VstpError::Timeout) => Ok(()),
            other => panic!("Expected timeout error, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_serialization_error() -> Result<(), VstpError> {
        let server = VstpServer::bind_tcp("127.0.0.1:8084").await?;
        tokio::spawn(async move {
            server
                .serve(|msg: TestMessage| async move { Ok(msg) })
                .await
        });

        tokio::time::sleep(Duration::from_millis(100)).await;

        let client = VstpClient::connect_tcp("127.0.0.1:8084").await?;

        // Send invalid JSON data
        let frame = Frame::new(FrameType::Data).with_payload(b"invalid json".to_vec());
        client.send_raw(frame).await?;

        // Wait for error response
        tokio::time::sleep(Duration::from_millis(100)).await;

        match client.receive::<TestMessage>().await {
            Err(VstpError::Protocol(msg)) if msg.contains("Deserialization error") => Ok(()),
            other => panic!("Expected deserialization error, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_multiple_clients() -> Result<(), VstpError> {
        let server = VstpServer::bind_tcp("127.0.0.1:8085").await?;
        tokio::spawn(async move {
            server
                .serve(|msg: TestMessage| async move { Ok(msg) })
                .await
        });

        tokio::time::sleep(Duration::from_millis(100)).await;

        let mut clients = vec![];
        for _ in 0..5 {
            let client = VstpClient::connect_tcp("127.0.0.1:8085").await?;
            clients.push(client);
        }

        for (i, client) in clients.iter().enumerate() {
            let msg = TestMessage {
                content: format!("Message from client {}", i),
            };
            client.send(msg).await?;
        }

        for (i, client) in clients.iter().enumerate() {
            let response: TestMessage = client.receive().await?;
            assert_eq!(response.content, format!("Message from client {}", i));
        }

        Ok(())
    }

    #[tokio::test]
    async fn test_auto_client_connect_and_echo() -> Result<(), VstpError> {
        let server = VstpServer::bind_udp("127.0.0.1:8090").await?;
        tokio::spawn(async move { server.serve(|msg: TestMessage| async move { Ok(msg) }).await });
        tokio::time::sleep(Duration::from_millis(100)).await;

        let mut cfg = AutoSwitchConfig::default();
        cfg.probe_attempts = 1;
        let client = VstpClient::connect_auto_with_config("127.0.0.1:8090", cfg).await?;
        let msg = TestMessage {
            content: "auto mode".to_string(),
        };
        client.send(msg.clone()).await?;
        let response: TestMessage = client.receive().await?;
        assert_eq!(response, msg);
        Ok(())
    }
}
