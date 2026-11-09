use thiserror::Error;

#[derive(Error, Debug)]
pub enum NetError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("TLS error: {0}")]
    Tls(String),

    #[error("RFC 2217 protocol error: {0}")]
    Rfc2217(String),

    #[error("Connection closed unexpectedly")]
    Closed,

    #[error("Timeout awaiting network response")]
    Timeout,
}

pub type Result<T> = std::result::Result<T, NetError>;
