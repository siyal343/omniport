//! POSIX PTY, termios token-bucket baud limiter, and Linux CUSE backend.

pub mod error;
pub mod pty;
pub mod symlink;
pub mod termios;
pub mod throttler;

pub use error::{HalError, Result};
pub use pty::PtyEndpoint;
pub use symlink::{SymlinkGuard, SymlinkManager};
pub use termios::TermiosPoller;
pub use throttler::{TokenBucketThrottler, calculate_byte_rate};
