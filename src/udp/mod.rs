//! UDP transport implementation for VSTP
//!
//! This module provides async UDP client and server implementations with
//! fragmentation, CRC validation, and optional ACK reliability.

pub mod client;
pub mod server;
pub mod reassembly;

pub use client::VstpUdpClient;
pub use server::VstpUdpServer;
