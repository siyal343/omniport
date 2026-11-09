use std::path::PathBuf;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum HalError {
    #[error("failed to open ptmx device: {0}")]
    OpenPt(#[source] std::io::Error),

    #[error("failed to grantpt: {0}")]
    GrantPt(#[source] std::io::Error),

    #[error("failed to unlockpt: {0}")]
    UnlockPt(#[source] std::io::Error),

    #[error("failed to get ptsname: {0}")]
    PtsName(String),

    #[error("failed to create symlink from '{link}' to '{target}': {source}")]
    Symlink {
        link: PathBuf,
        target: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("termios operation failed: {0}")]
    Termios(#[source] std::io::Error),

    #[error("invalid or unsupported baud rate: {0}")]
    UnsupportedBaud(u32),

    #[error("failed to open cuse device (/dev/cuse): {0}")]
    OpenCuse(#[source] std::io::Error),

    #[error("CUSE initialization failed: {0}")]
    CuseInit(String),

    #[error("unsupported or invalid ioctl command: 0x{0:X}")]
    UnsupportedIoctl(u32),

    #[error("ioctl argument error: {0}")]
    InvalidIoctlArg(String),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, HalError>;
