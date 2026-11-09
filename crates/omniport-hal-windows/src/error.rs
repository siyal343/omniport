use thiserror::Error;

#[derive(Error, Debug)]
pub enum HalError {
    #[error("failed to open serial port '{port}': {source}")]
    OpenPort {
        port: String,
        #[source]
        source: std::io::Error,
    },

    #[error("failed to get/set DCB comm state: {0}")]
    CommState(#[source] std::io::Error),

    #[error("failed to set comm timeouts: {0}")]
    CommTimeouts(#[source] std::io::Error),

    #[error("failed to set comm mask / wait event: {0}")]
    CommEvent(#[source] std::io::Error),

    #[error("overlapped I/O operation failed: {0}")]
    OverlappedIo(#[source] std::io::Error),

    #[error("modem status query failed: {0}")]
    ModemStatus(#[source] std::io::Error),

    #[error("escape comm function failed: {0}")]
    EscapeComm(#[source] std::io::Error),

    #[error("com0com virtual bus driver is not installed or accessible: {0}")]
    Com0ComDriverNotFound(String),

    #[error("com0com operation failed: {0}")]
    Com0Com(String),

    #[error("invalid serial port parameter: {0}")]
    InvalidParameter(String),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, HalError>;
