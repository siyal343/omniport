//! Network serial port endpoints and RFC 2217 protocol sync for OmniPort.
//!
//! Provides TCP client/server endpoints, UDP datagram endpoints, mutual TLS (mTLS)
//! endpoints via `rustls`, and full RFC 2217 Telnet Com Port Control Option synchronization.

pub mod error;
pub mod rfc2217;
pub mod tcp;
pub mod tls;
pub mod udp;

pub use error::{NetError, Result};
pub use rfc2217::{Rfc2217Codec, Rfc2217Command};
pub use tcp::TcpEndpoint;
pub use tls::TlsEndpoint;
pub use udp::UdpEndpoint;
