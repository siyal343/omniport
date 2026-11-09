use thiserror::Error;

#[derive(Error, Debug)]
pub enum ProtoError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("serialization/deserialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("frame size {size} exceeds maximum permitted frame size {max}")]
    FrameTooLarge { size: usize, max: usize },

    #[error("unexpected response from server: {0}")]
    UnexpectedResponse(String),

    #[error("server returned error: {0}")]
    ServerError(String),

    #[error("remote connection closed unexpectedly")]
    ConnectionClosed,
}

pub type Result<T> = std::result::Result<T, ProtoError>;
