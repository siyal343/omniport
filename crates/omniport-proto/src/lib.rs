//! Control-plane RPC protocol definitions and client for omniportd.

pub mod client;
pub mod codec;
pub mod error;
pub mod types;

pub use client::RpcClient;
pub use codec::{MAX_FRAME_SIZE, read_message, write_message};
pub use error::{ProtoError, Result};
pub use types::{EndpointInfo, Request, Response, SerialConfigDto};
