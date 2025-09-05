use crate::{Flags, Frame, FrameType, VstpError};
use serde::{de::DeserializeOwned, Serialize};
use std::{net::SocketAddr, sync::Arc, time::Duration};
use tokio::sync::{mpsc, Mutex};

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

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

    /// Set operation timeout
    pub fn set_timeout(&mut self, timeout: Duration) {
        self.timeout = timeout;
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
}

struct ServerMessage {
    data: Vec<u8>,
    client_addr: SocketAddr,
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
                                let (response_tx, mut response_rx) = mpsc::channel(1);

                                // Try to deserialize and handle the message
                                match serde_json::from_slice::<T>(&frame.payload()) {
                                    Ok(data) => {
                                        if let Err(_) = tokio::time::timeout(
                                            timeout,
                                            tx.send(ServerMessage {
                                                data: frame.payload().to_vec(),
                                                client_addr: client.peer_addr(),
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
                        let (response_tx, mut response_rx) = mpsc::channel(1);

                        // Try to deserialize and handle the message
                        match serde_json::from_slice::<T>(&frame.payload()) {
                            Ok(data) => {
                                if let Err(_) = tokio::time::timeout(
                                    timeout,
                                    tx.send(ServerMessage {
                                        data: frame.payload().to_vec(),
                                        client_addr: addr,
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
}
