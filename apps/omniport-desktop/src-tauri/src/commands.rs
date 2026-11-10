use crate::capture_store::{CaptureStatus, CaptureStore, DisconnectedCaptureStore};
use std::sync::Arc;
use tauri::{State, ipc::Response};

/// Re-exported so the GUI/Tauri layer shares the capture crate's single limit.
pub use omniport_capture::viewport::MAX_VIEWPORT_ROWS;

pub struct AppCaptureState {
    store: Arc<dyn CaptureStore>,
}

impl AppCaptureState {
    pub fn new(store: Arc<dyn CaptureStore>) -> Self {
        Self { store }
    }

    pub fn disconnected(reason: impl Into<String>) -> Self {
        Self::new(Arc::new(DisconnectedCaptureStore::new(reason)))
    }

    pub async fn read_viewport(&self, start_row: u64, row_count: usize) -> Result<Vec<u8>, String> {
        self.store.read_viewport(start_row, row_count).await
    }

    pub async fn status(&self) -> Result<CaptureStatus, String> {
        self.store.status().await
    }
}

/// Tauri IPC command to fetch a visible row slice from the active `CaptureStore`.
/// Returns raw binary bytes according to ICD Section 2 without JSON overhead.
#[tauri::command]
pub async fn fetch_viewport_rows(
    state: State<'_, AppCaptureState>,
    start_row: u64,
    row_count: usize,
) -> Result<Response, String> {
    let raw_wire_buf = state.read_viewport(start_row, row_count).await?;
    Ok(Response::new(raw_wire_buf))
}

/// Tauri IPC command describing the capture session behind `fetch_viewport_rows`.
#[tauri::command]
pub async fn capture_status(state: State<'_, AppCaptureState>) -> Result<CaptureStatus, String> {
    state.status().await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capture_store::FixtureCaptureStore;
    use bytes::Bytes;
    use omniport_capture::MmapCaptureEngine;
    use omniport_core::{FrameFlags, PinState, SerialEvent};
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_app_capture_state_wire_format_and_slicing() {
        let dir = tempdir().unwrap();
        let mut engine = MmapCaptureEngine::create(dir.path(), 50, 4096).unwrap();

        for i in 0..3 {
            let evt = SerialEvent {
                timestamp_ns: 1_000_000 + i,
                source_id: (10 + i) as u16,
                flags: FrameFlags::empty(),
                pins: PinState::DTR | PinState::RTS,
                payload: Bytes::copy_from_slice(format!("PACKET_{}", i).as_bytes()),
            };
            engine.append_event(&evt);
        }

        let state = AppCaptureState::new(Arc::new(FixtureCaptureStore::from_engine(engine)));

        let wire = state.read_viewport(0, 10).await.unwrap();
        assert!(wire.len() >= 16);

        let total = u64::from_le_bytes(wire[0..8].try_into().unwrap());
        let returned = u32::from_le_bytes(wire[8..12].try_into().unwrap());
        assert_eq!(total, 3);
        assert_eq!(returned, 3);

        let mut offset = 16;
        for i in 0..3 {
            let ts = u64::from_le_bytes(wire[offset..offset + 8].try_into().unwrap());
            assert_eq!(ts, 1_000_000 + i);

            let payload_len =
                u32::from_le_bytes(wire[offset + 16..offset + 20].try_into().unwrap()) as usize;
            let ep_id = u16::from_le_bytes(wire[offset + 20..offset + 22].try_into().unwrap());
            let pins = wire[offset + 23];

            assert_eq!(ep_id, (10 + i) as u16);
            assert_eq!(pins, (PinState::DTR | PinState::RTS).bits());

            let p_start = offset + 24;
            let payload = &wire[p_start..p_start + payload_len];
            assert_eq!(payload, format!("PACKET_{}", i).as_bytes());

            offset = p_start + payload_len;
        }

        let slice_wire = state.read_viewport(1, 1).await.unwrap();
        let slice_total = u64::from_le_bytes(slice_wire[0..8].try_into().unwrap());
        let slice_returned = u32::from_le_bytes(slice_wire[8..12].try_into().unwrap());
        assert_eq!(slice_total, 3);
        assert_eq!(slice_returned, 1);

        let slice_ep_id = u16::from_le_bytes(slice_wire[16 + 20..16 + 22].try_into().unwrap());
        assert_eq!(slice_ep_id, 11);
    }

    #[tokio::test]
    async fn test_app_capture_state_out_of_bounds() {
        let dir = tempdir().unwrap();
        let engine = MmapCaptureEngine::create(dir.path(), 50, 4096).unwrap();
        let state = AppCaptureState::new(Arc::new(FixtureCaptureStore::from_engine(engine)));

        let wire = state.read_viewport(100, 10).await.unwrap();
        let total = u64::from_le_bytes(wire[0..8].try_into().unwrap());
        let returned = u32::from_le_bytes(wire[8..12].try_into().unwrap());
        assert_eq!(total, 0);
        assert_eq!(returned, 0);
    }

    #[tokio::test]
    async fn test_app_capture_state_rejects_absurd_row_count_without_overallocating() {
        let dir = tempdir().unwrap();
        let mut engine = MmapCaptureEngine::create(dir.path(), 50, 4096).unwrap();
        for i in 0..3 {
            engine.append_event(&SerialEvent {
                timestamp_ns: i,
                source_id: 1,
                flags: FrameFlags::empty(),
                pins: PinState::empty(),
                payload: Bytes::from_static(b"X"),
            });
        }
        let state = AppCaptureState::new(Arc::new(FixtureCaptureStore::from_engine(engine)));

        let wire = state.read_viewport(0, usize::MAX).await.unwrap();
        let returned = u32::from_le_bytes(wire[8..12].try_into().unwrap());
        assert_eq!(returned, 3);

        let wire = state.read_viewport(u64::MAX, usize::MAX).await.unwrap();
        let returned = u32::from_le_bytes(wire[8..12].try_into().unwrap());
        assert_eq!(returned, 0);
    }

    #[tokio::test]
    async fn test_app_capture_state_clamps_to_max_rows() {
        let dir = tempdir().unwrap();
        let max_packets = MAX_VIEWPORT_ROWS as u64 + 100;
        let mut engine =
            MmapCaptureEngine::create(dir.path(), max_packets, max_packets * 4).unwrap();
        for i in 0..max_packets {
            engine.append_event(&SerialEvent {
                timestamp_ns: i,
                source_id: 1,
                flags: FrameFlags::empty(),
                pins: PinState::empty(),
                payload: Bytes::from_static(b"AB"),
            });
        }
        let state = AppCaptureState::new(Arc::new(FixtureCaptureStore::from_engine(engine)));

        let wire = state.read_viewport(0, usize::MAX).await.unwrap();
        let total = u64::from_le_bytes(wire[0..8].try_into().unwrap());
        let returned = u32::from_le_bytes(wire[8..12].try_into().unwrap());
        assert_eq!(total, max_packets);
        assert_eq!(returned, MAX_VIEWPORT_ROWS as u32);
    }

    #[tokio::test]
    async fn test_app_capture_state_disconnected_surfaces_error() {
        let state = AppCaptureState::disconnected("host capture store unavailable");
        assert_eq!(
            state.read_viewport(0, 10).await.unwrap_err(),
            "host capture store unavailable"
        );
        assert_eq!(
            state.status().await.unwrap_err(),
            "host capture store unavailable"
        );
    }
}
