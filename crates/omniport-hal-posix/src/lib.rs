//! POSIX PTY, termios token-bucket baud limiter, and Linux CUSE backend.

pub mod cuse;
pub mod error;
pub mod pty;
pub mod symlink;
pub mod termios;

pub use cuse::{
    CuseEndpoint, KERNEL_TERMIOS_SIZE, KERNEL_TERMIOS2_SIZE, KernelTermios, SERIAL_ICOUNTER_SIZE,
    SerialCounters, SerialIoctlHandler, TCGETS, TCGETS2, TCSBRK, TCSETS, TCSETS2, TCSETSF,
    TCSETSF2, TCSETSW, TCSETSW2, TIOCGICOUNT, TIOCM_CAR, TIOCM_CD, TIOCM_CTS, TIOCM_DSR, TIOCM_DTR,
    TIOCM_LE, TIOCM_RI, TIOCM_RNG, TIOCM_RTS, TIOCMBIC, TIOCMBIS, TIOCMGET, TIOCMIWAIT, TIOCMSET,
    TIOCSBRK, pin_state_to_tiocm, tiocm_to_pin_state,
};
pub use error::{HalError, Result};
pub use omniport_core::{TokenBucketThrottler, calculate_byte_rate};
pub use pty::PtyEndpoint;
pub use symlink::{SymlinkGuard, SymlinkManager};
pub use termios::TermiosPoller;
