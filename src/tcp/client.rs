use futures::SinkExt;
use tokio::net::TcpStream;
use tokio_stream::StreamExt;
use tokio_util::codec::{FramedRead, FramedWrite};
use tracing::{debug, info};

use crate::types::{Frame, FrameType, VstpError};
use crate::VstpFrameCodec as Codec;

/// TCP client for VSTP protocol
pub struct VstpTcpClient {
    framed_write: FramedWrite<tokio::net::tcp::OwnedWriteHalf, Codec>,
    framed_read: FramedRead<tokio::net::tcp::OwnedReadHalf, Codec>,
}

impl VstpTcpClient {
    /// Connect to a VSTP server
    pub async fn connect(addr: &str) -> Result<Self, VstpError> {
        let socket = TcpStream::connect(addr).await?;
        info!("Connected to VSTP server at {}", addr);

        let (read, write) = socket.into_split();
        let framed_read = FramedRead::new(read, Codec::default());
        let framed_write = FramedWrite::new(write, Codec::default());

        Ok(Self {
            framed_write,
            framed_read,
        })
    }

    /// Send a frame to the server
    pub async fn send(&mut self, frame: Frame) -> Result<(), VstpError> {
        debug!("Sending frame: {:?}", frame.typ);
        self.framed_write.send(frame).await?;
        Ok(())
    }

    /// Receive a frame from the server
    pub async fn recv(&mut self) -> Result<Option<Frame>, VstpError> {
        let frame = self.framed_read.try_next().await?;
        if let Some(ref frame) = frame {
            debug!("Received frame: {:?}", frame.typ);
        }
        Ok(frame)
    }

    /// Close the connection gracefully
    pub async fn close(&mut self) -> Result<(), VstpError> {
        // Send BYE frame
        let bye_frame = Frame::new(FrameType::Bye);
        self.send(bye_frame).await?;

        // Close the write half
        self.framed_write.close().await?;

        info!("Connection closed gracefully");
        Ok(())
    }

    /// Send a HELLO frame to start the session
    pub async fn send_hello(&mut self) -> Result<(), VstpError> {
        let hello_frame = Frame::new(FrameType::Hello);
        self.send(hello_frame).await
    }

    /// Send a DATA frame with the given payload
    pub async fn send_data(&mut self, payload: Vec<u8>) -> Result<(), VstpError> {
        let data_frame = Frame::new(FrameType::Data).with_payload(payload);
        self.send(data_frame).await
    }
}
