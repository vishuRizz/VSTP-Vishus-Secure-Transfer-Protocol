use futures::SinkExt;
use std::future::Future;
use std::sync::Arc;
use tokio::net::{TcpListener, TcpStream, ToSocketAddrs};
use tokio::sync::Mutex;
use tokio_stream::StreamExt;
use tokio_util::codec::Framed;
use tracing::info;

use crate::types::{Frame, SessionId, VstpError};
use crate::VstpFrameCodec as Codec;

/// TCP connection handler
pub struct VstpTcpConnection {
    framed: Framed<TcpStream, Codec>,
    session_id: SessionId,
    peer_addr: std::net::SocketAddr,
}

impl VstpTcpConnection {
    /// Send a frame to the client
    pub async fn send(&mut self, frame: Frame) -> Result<(), VstpError> {
        self.framed.send(frame).await?;
        Ok(())
    }

    /// Receive a frame from the client
    pub async fn recv(&mut self) -> Result<Option<Frame>, VstpError> {
        let frame = self.framed.next().await.transpose()?;
        Ok(frame)
    }

    /// Get the peer address
    pub fn peer_addr(&self) -> std::net::SocketAddr {
        self.peer_addr
    }
}

/// TCP server for VSTP protocol
pub struct VstpTcpServer {
    listener: TcpListener,
    next_session_id: Arc<Mutex<u128>>,
}

impl VstpTcpServer {
    /// Bind to the specified address
    pub async fn bind(addr: impl ToSocketAddrs) -> Result<Self, VstpError> {
        let listener = TcpListener::bind(addr).await?;
        info!("VSTP TCP server bound to {}", listener.local_addr()?);

        Ok(Self {
            listener,
            next_session_id: Arc::new(Mutex::new(1)),
        })
    }

    /// Accept a new client connection
    pub async fn accept(&self) -> Result<VstpTcpConnection, VstpError> {
        let (socket, addr) = self.listener.accept().await?;
        let session_id = {
            let mut id_guard = self.next_session_id.lock().await;
            *id_guard += 1;
            *id_guard
        };

        info!("New connection from {} (session {})", addr, session_id);

        Ok(VstpTcpConnection {
            framed: Framed::new(socket, Codec::default()),
            session_id,
            peer_addr: addr,
        })
    }

    /// Get the local address this server is bound to
    pub fn local_addr(&self) -> Result<std::net::SocketAddr, VstpError> {
        self.listener.local_addr().map_err(|e| VstpError::Io(e))
    }

    /// Run the server with the provided handler function
    pub async fn run<F, Fut>(self, handler: F) -> Result<(), VstpError>
    where
        F: Fn(SessionId, Frame) -> Fut + Send + Sync + Clone + 'static,
        Fut: Future<Output = ()> + Send,
    {
        info!("VSTP TCP server starting...");

        loop {
            match self.accept().await {
                Ok(mut conn) => {
                    let handler = handler.clone();
                    let session_id = conn.session_id;

                    tokio::spawn(async move {
                        while let Ok(Some(frame)) = conn.recv().await {
                            handler(session_id, frame).await;
                        }
                        info!("Session {} ended", session_id);
                    });
                }
                Err(e) => {
                    tracing::error!("Failed to accept connection: {}", e);
                }
            }
        }
    }
}
