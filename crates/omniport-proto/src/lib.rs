//! Control-plane RPC protocol definitions and client for omniportd.
//!
//! # Wire transport (current)
//!
//! The control plane is **not** gRPC yet. [`RpcClient`](crate::RpcClient) talks to
//! `omniportd` over TCP using a 4-byte big-endian length prefix followed by a
//! UTF-8 JSON payload (see [`codec`]); the [`Request`](crate::Request) and
//! [`Response`](crate::Response) enums in [`types`] are the authoritative
//! schema. No build script compiles either `.proto` file in this repository:
//!
//! * `crates/omniport-proto/proto/omniport.proto` (`package omniport`, service
//!   `OmniPortControl`, 7 RPCs) is the design reference for the JSON messages
//!   but is not generated or used at runtime.
//! * `proto/omniport.proto` at the repository root (`package omniport.v1`,
//!   service `OmniPortDaemon`, 2 RPCs) is a stale early draft: its message
//!   names/fields (`name_a`/`name_b`, `kind`, `rx_bytes`) do not match
//!   [`types`](crate::types).
//!
//! Treat `types.rs` as canonical until an actual gRPC/tonic transport replaces
//! the JSON codec; do not generate code from either proto before then.

pub mod client;
pub mod codec;
pub mod error;
pub mod types;

pub use client::RpcClient;
pub use codec::{MAX_FRAME_SIZE, read_message, write_message};
pub use error::{ProtoError, Result};
pub use types::{EndpointInfo, Request, Response, SerialConfigDto};
