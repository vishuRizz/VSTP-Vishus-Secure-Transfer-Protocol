use std::future::Future;
use std::sync::Arc;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Mutex;
use tokio_stream::StreamExt;
use tokio_util::codec::Framed;
use tracing::{error, info};

use crate::types::{Frame, SessionId, VstpError};
use crate::VstpFrameCodec as Codec;

/// TCP server for VSTP protocol
pub struct VstpTcpServer {
    listener: TcpListener,
    next_session_id: Arc<Mutex<u128>>,
}

impl VstpTcpServer {
    /// Bind to the specified address
    pub async fn bind(addr: &str) -> Result<Self, VstpError> {
        let listener = TcpListener::bind(addr).await?;
        info!("VSTP TCP server bound to {}", addr);

        Ok(Self {
            listener,
            next_session_id: Arc::new(Mutex::new(1)),
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
            match self.listener.accept().await {
                Ok((socket, addr)) => {
                    info!("New connection from {}", addr);

                    let handler = handler.clone();
                    let next_session_id = self.next_session_id.clone();

                    tokio::spawn(async move {
                        if let Err(e) =
                            Self::handle_connection(socket, handler, next_session_id).await
                        {
                            error!("Connection handler error: {}", e);
                        }
                    });
                }
                Err(e) => {
                    error!("Failed to accept connection: {}", e);
                }
            }
        }
    }

    /// Handle a single client connection
    async fn handle_connection<F, Fut>(
        socket: TcpStream,
        handler: F,
        next_session_id: Arc<Mutex<u128>>,
    ) -> Result<(), VstpError>
    where
        F: Fn(SessionId, Frame) -> Fut + Send + Sync + Clone + 'static,
        Fut: Future<Output = ()> + Send,
    {
        // Generate session ID
        let session_id = {
            let mut id_guard = next_session_id.lock().await;
            *id_guard += 1;
            *id_guard
        };

        info!("Starting session {}", session_id);

        // Create framed stream
        let mut framed = Framed::new(socket, Codec::default());

        // Handle incoming frames
        loop {
            match framed.try_next().await {
                Ok(Some(frame)) => {
                    info!("Session {} received frame: {:?}", session_id, frame.typ);
                    // Call user handler
                    handler(session_id, frame).await;
                }
                Ok(None) => {
                    info!("Session {} connection closed", session_id);
                    break;
                }
                Err(e) => {
                    error!("Session {} frame error: {}", session_id, e);
                    break;
                }
            }
        }

        info!("Session {} ended", session_id);
        Ok(())
    }
}
