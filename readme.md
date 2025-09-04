# VSTP - Vishu's Secure Transfer Protocol

[![Crates.io](https://img.shields.io/crates/v/vstp.svg)](https://crates.io/crates/vstp)
[![Documentation](https://docs.rs/vstp/badge.svg)](https://docs.rs/vstp)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](https://github.com/yourusername/vstp#license)

A fast, secure, and extensible binary protocol for TCP and UDP communication in Rust.

## Features

- 🚀 **Dual Transport Support**: Choose between reliable TCP or fast UDP
- 🔒 **Security Ready**: TLS 1.3 support for TCP (coming in v0.2)
- 📦 **Fragmentation**: Automatic handling of large payloads over UDP
- ✅ **Reliability**: Optional ACK-based reliability for UDP
- 🏗️ **Extensible**: Binary headers for custom metadata
- ⚡ **High Performance**: Zero-copy operations and async/await support
- 🧪 **Well Tested**: Comprehensive test suite and examples

## Quick Start

Add VSTP to your `Cargo.toml`:

```toml
[dependencies]
vstp = "0.1"
tokio = { version = "1.0", features = ["full"] }
```