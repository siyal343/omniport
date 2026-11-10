//! Control-plane RPC protocol definitions and client for `omniportd`.
//!
//! # Wire transport
//!
//! [`RpcClient`](crate::RpcClient) communicates with `omniportd` over TCP using
//! a 4-byte big-endian length prefix followed by a UTF-8 JSON payload
//! (see [`codec`] and `docs/ICD.md` §3). The [`Request`](crate::Request) and
//! [`Response`](crate::Response) enums in [`types`] are the authoritative
//! schema.

pub mod client;
pub mod codec;
pub mod error;
pub mod types;

pub use client::RpcClient;
pub use codec::{MAX_FRAME_SIZE, read_message, write_message};
pub use error::{ProtoError, Result};
pub use types::{
    EndpointInfo, MAX_VIEWPORT_BATCH_BYTES, Request, Response, Route, SerialConfigDto,
};
