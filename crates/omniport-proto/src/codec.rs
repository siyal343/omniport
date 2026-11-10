//! Length-delimited JSON framing used on the daemon control plane (see `docs/ICD.md` §3).
//!
//! Wire format: `[len: u32 big-endian][serde_json bytes]`.

use crate::error::{ProtoError, Result};
use serde::Serialize;
use serde::de::DeserializeOwned;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

pub const MAX_FRAME_SIZE: usize = 16 * 1024 * 1024; // 16 MB limit

/// Encodes and writes a length-delimited JSON payload
pub async fn write_message<W, T>(writer: &mut W, msg: &T) -> Result<()>
where
    W: AsyncWriteExt + Unpin,
    T: Serialize,
{
    let bytes = serde_json::to_vec(msg)?;
    if bytes.len() > MAX_FRAME_SIZE {
        return Err(ProtoError::FrameTooLarge {
            size: bytes.len(),
            max: MAX_FRAME_SIZE,
        });
    }

    let len = bytes.len() as u32;
    writer.write_all(&len.to_be_bytes()).await?;
    writer.write_all(&bytes).await?;
    writer.flush().await?;
    Ok(())
}

/// Reads and decodes a length-delimited JSON payload
pub async fn read_message<R, T>(reader: &mut R) -> Result<T>
where
    R: AsyncReadExt + Unpin,
    T: DeserializeOwned,
{
    let mut len_buf = [0u8; 4];
    match reader.read_exact(&mut len_buf).await {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
            return Err(ProtoError::ConnectionClosed);
        }
        Err(e) => return Err(ProtoError::Io(e)),
    }

    let len = u32::from_be_bytes(len_buf) as usize;
    if len > MAX_FRAME_SIZE {
        return Err(ProtoError::FrameTooLarge {
            size: len,
            max: MAX_FRAME_SIZE,
        });
    }

    let mut buf = vec![0u8; len];
    reader.read_exact(&mut buf).await?;
    let msg: T = serde_json::from_slice(&buf)?;
    Ok(msg)
}
