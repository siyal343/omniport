//! OmniPort Headless Daemon (`omniportd`) control-plane engine and RPC server.

pub mod daemon;
pub mod server;

pub use daemon::DaemonEngine;
pub use server::{
    DEFAULT_BIND_ADDR, DEFAULT_IDLE_TIMEOUT, DEFAULT_MAX_CONNECTIONS,
    DEFAULT_MAX_REQUESTS_PER_SECOND, DEFAULT_REQUEST_TIMEOUT, ServerConfig, bind_is_loopback,
    run_server, run_server_with_config,
};
