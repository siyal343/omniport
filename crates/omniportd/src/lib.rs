//! OmniPort Headless Daemon (`omniportd`) control-plane engine and RPC server.

pub mod daemon;
pub mod server;

pub use daemon::DaemonEngine;
pub use server::run_server;
