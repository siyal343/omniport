use omniport_capture::MmapCaptureEngine;
use omniport_capture::viewport::{MAX_VIEWPORT_ROWS, ViewportBatch};
use omniport_proto::{ProtoError, RpcClient};
use std::future::Future;
use std::pin::Pin;
use std::sync::RwLock;
use std::time::Duration;

const FIXTURE_MAX_PACKETS: u64 = 100_000;
const FIXTURE_MAX_DATA_BYTES: u64 = 16 * 1024 * 1024;
pub(crate) const DAEMON_CONNECT_TIMEOUT: Duration = Duration::from_secs(2);
pub const DEFAULT_DAEMON_ADDR: &str = "127.0.0.1:50051";

/// Non-viewport metadata the frontend needs to describe the capture it is reading.
///
/// `demo_mode` lets the UI label a seeded/fixture session as demo data instead of
/// presenting it as live hardware traffic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct CaptureStatus {
    pub demo_mode: bool,
    pub total_packets: u64,
    pub dropped_frames: u64,
}

pub type StoreFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, String>> + Send + 'a>>;

/// Single seam between the Tauri IPC commands and the capture backing store.
///
/// Implementations either read from `omniportd` over `omniport-proto`
/// ([`DaemonCaptureStore`]) or serve an explicit dev/demo fixture
/// ([`FixtureCaptureStore`]).
pub trait CaptureStore: Send + Sync {
    /// Returns an ICD §2 encoded `ViewportBatch` for `[start_row .. start_row + row_count)`.
    fn read_viewport(&self, start_row: u64, row_count: usize) -> StoreFuture<'_, Vec<u8>>;

    /// Reports session metadata (`demo_mode`, `total_packets`, `dropped_frames`).
    fn status(&self) -> StoreFuture<'_, CaptureStatus>;
}

/// Live capture adapter communicating with `omniportd` over `omniport-proto`.
pub struct DaemonCaptureStore {
    addr: String,
    client: tokio::sync::Mutex<Option<RpcClient>>,
}

impl DaemonCaptureStore {
    pub fn new(addr: impl Into<String>) -> Self {
        Self {
            addr: addr.into(),
            client: tokio::sync::Mutex::new(None),
        }
    }

    pub fn addr(&self) -> &str {
        &self.addr
    }

    async fn connect(addr: &str) -> Result<RpcClient, String> {
        tokio::time::timeout(DAEMON_CONNECT_TIMEOUT, RpcClient::connect(addr))
            .await
            .map_err(|_| format!("Timed out connecting to omniportd at {addr}"))?
            .map_err(|e| format!("Failed to connect to omniportd at {addr}: {e}"))
    }

    async fn read_viewport_inner(
        &self,
        start_row: u64,
        row_count: usize,
    ) -> Result<Vec<u8>, String> {
        let clamped_count = row_count.min(MAX_VIEWPORT_ROWS) as u32;
        let mut guard = self.client.lock().await;
        let had_cached = guard.is_some();
        if guard.is_none() {
            *guard = Some(Self::connect(&self.addr).await?);
        }

        let wire = match guard
            .as_mut()
            .expect("client connected")
            .read_viewport(start_row, clamped_count)
            .await
        {
            Ok(bytes) => bytes,
            Err(ProtoError::ConnectionClosed | ProtoError::Io(_)) if had_cached => {
                *guard = None;
                *guard = Some(Self::connect(&self.addr).await?);
                guard
                    .as_mut()
                    .expect("client reconnected")
                    .read_viewport(start_row, clamped_count)
                    .await
                    .map_err(|e| {
                        *guard = None;
                        format!("omniportd viewport read failed: {e}")
                    })?
            }
            Err(err) => {
                *guard = None;
                return Err(format!("omniportd viewport read failed: {err}"));
            }
        };

        ViewportBatch::decode(&wire)
            .map_err(|e| format!("invalid viewport batch from omniportd: {e}"))?;
        Ok(wire)
    }

    async fn status_inner(&self) -> Result<CaptureStatus, String> {
        let mut guard = self.client.lock().await;
        let had_cached = guard.is_some();
        if guard.is_none() {
            *guard = Some(Self::connect(&self.addr).await?);
        }

        let (total_packets, dropped_frames) = match guard
            .as_mut()
            .expect("client connected")
            .capture_status()
            .await
        {
            Ok(stats) => stats,
            Err(ProtoError::ConnectionClosed | ProtoError::Io(_)) if had_cached => {
                *guard = None;
                *guard = Some(Self::connect(&self.addr).await?);
                guard
                    .as_mut()
                    .expect("client reconnected")
                    .capture_status()
                    .await
                    .map_err(|e| {
                        *guard = None;
                        format!("omniportd capture_status failed: {e}")
                    })?
            }
            Err(err) => {
                *guard = None;
                return Err(format!("omniportd capture_status failed: {err}"));
            }
        };

        Ok(CaptureStatus {
            demo_mode: false,
            total_packets,
            dropped_frames,
        })
    }
}

impl CaptureStore for DaemonCaptureStore {
    fn read_viewport(&self, start_row: u64, row_count: usize) -> StoreFuture<'_, Vec<u8>> {
        Box::pin(self.read_viewport_inner(start_row, row_count))
    }

    fn status(&self) -> StoreFuture<'_, CaptureStatus> {
        Box::pin(self.status_inner())
    }
}

/// Explicit dev/demo/test adapter backed by an in-process `MmapCaptureEngine`.
///
/// Only selected for debug dev runs (`tauri dev`) or explicit
/// `OMNIPORT_DESKTOP_DEMO=1` sessions; release builds default to
/// [`DaemonCaptureStore`].
pub struct FixtureCaptureStore {
    // Drop `engine` before `_dir` so Windows unmaps files before TempDir cleanup.
    engine: RwLock<MmapCaptureEngine>,
    _dir: Option<tempfile::TempDir>,
}

impl FixtureCaptureStore {
    /// Creates an isolated in-process capture store preloaded with 25 deterministic
    /// demo frames for standalone UI development without `omniportd`.
    pub fn seeded() -> std::io::Result<Self> {
        let dir = tempfile::tempdir()?;
        let mut engine =
            MmapCaptureEngine::create(dir.path(), FIXTURE_MAX_PACKETS, FIXTURE_MAX_DATA_BYTES)?;
        seed_demo_session(&mut engine);
        Ok(Self {
            engine: RwLock::new(engine),
            _dir: Some(dir),
        })
    }

    /// Wraps an existing `MmapCaptureEngine` as a fixture store (for unit tests).
    pub fn from_engine(engine: MmapCaptureEngine) -> Self {
        Self {
            engine: RwLock::new(engine),
            _dir: None,
        }
    }
}

impl CaptureStore for FixtureCaptureStore {
    fn read_viewport(&self, start_row: u64, row_count: usize) -> StoreFuture<'_, Vec<u8>> {
        Box::pin(async move {
            let guard = self.engine.read().map_err(|e| e.to_string())?;
            Ok(guard
                .read_viewport_batch(start_row, row_count)
                .encode_to_vec())
        })
    }

    fn status(&self) -> StoreFuture<'_, CaptureStatus> {
        Box::pin(async move {
            let guard = self.engine.read().map_err(|e| e.to_string())?;
            Ok(CaptureStatus {
                demo_mode: true,
                total_packets: guard.packet_count(),
                dropped_frames: guard.dropped_frames(),
            })
        })
    }
}

/// Explicit error adapter used when host capture initialization fails or
/// `OMNIPORT_DESKTOP_DISCONNECT` is active.
pub struct DisconnectedCaptureStore {
    reason: String,
}

impl DisconnectedCaptureStore {
    pub fn new(reason: impl Into<String>) -> Self {
        Self {
            reason: reason.into(),
        }
    }
}

impl CaptureStore for DisconnectedCaptureStore {
    fn read_viewport(&self, _start_row: u64, _row_count: usize) -> StoreFuture<'_, Vec<u8>> {
        let reason = self.reason.clone();
        Box::pin(async move { Err(reason) })
    }

    fn status(&self) -> StoreFuture<'_, CaptureStatus> {
        let reason = self.reason.clone();
        Box::pin(async move { Err(reason) })
    }
}

fn seed_demo_session(engine: &mut MmapCaptureEngine) {
    let base_time = 1_700_000_000_000_000_000u64;
    for i in 0..25 {
        let is_tx = i % 2 == 0;
        let evt = omniport_core::SerialEvent {
            timestamp_ns: base_time + i * 5_000_000,
            source_id: if is_tx { 1 } else { 2 },
            flags: if is_tx {
                omniport_core::FrameFlags::DIR_TX
            } else {
                omniport_core::FrameFlags::empty()
            },
            pins: omniport_core::PinState::RTS | omniport_core::PinState::DTR,
            payload: bytes::Bytes::copy_from_slice(
                format!("OMNIPORT_INITIAL_FRAME_{:03}\r\n", i).as_bytes(),
            ),
        };
        engine.append_event(&evt);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_fixture_capture_store_seeded_is_deterministic_and_labelled_demo() {
        let store = FixtureCaptureStore::seeded().expect("fixture store initializes");
        let status = store.status().await.unwrap();
        assert!(status.demo_mode);
        assert_eq!(status.total_packets, 25);
        assert_eq!(status.dropped_frames, 0);

        let wire = store.read_viewport(0, 1).await.unwrap();
        let batch = ViewportBatch::decode(&wire).unwrap();
        assert_eq!(batch.total_packets(), 25);
        assert_eq!(batch.returned_rows(), 1);
        assert_eq!(batch.rows()[0].payload, b"OMNIPORT_INITIAL_FRAME_000\r\n");
    }

    #[tokio::test]
    async fn test_disconnected_capture_store_surfaces_error() {
        let store = DisconnectedCaptureStore::new("host capture store unavailable");
        assert_eq!(
            store.read_viewport(0, 10).await.unwrap_err(),
            "host capture store unavailable"
        );
        assert_eq!(
            store.status().await.unwrap_err(),
            "host capture store unavailable"
        );
    }
}
