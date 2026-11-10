use omniport_capture::MmapCaptureEngine;
use std::sync::RwLock;
use tauri::{ipc::Response, State};

/// Re-exported so the GUI/Tauri layer shares the capture crate's single limit.
pub use omniport_capture::viewport::MAX_VIEWPORT_ROWS;

/// Non-viewport metadata the frontend needs to describe the capture it is reading.
///
/// `demo_mode` lets the UI label a seeded/fixture session as demo data instead of
/// presenting it as live hardware traffic.
#[derive(Debug, Clone, Copy, serde::Serialize)]
pub struct CaptureStatus {
    pub demo_mode: bool,
    pub total_packets: u64,
    pub dropped_frames: u64,
}

pub struct AppCaptureState {
    pub engine: RwLock<MmapCaptureEngine>,
    pub demo_mode: bool,
}

/// Tauri IPC command to fetch a visible row slice from the dual-mmap capture store.
/// Returns raw binary bytes according to ICD Section 2 without JSON overhead.
#[tauri::command]
pub fn fetch_viewport_rows(
    state: State<'_, AppCaptureState>,
    start_row: u64,
    row_count: usize,
) -> Result<Response, String> {
    let engine = state.engine.read().map_err(|e| e.to_string())?;
    let raw_wire_buf = fetch_viewport_binary(&engine, start_row, row_count);
    Ok(Response::new(raw_wire_buf))
}

/// Tauri IPC command describing the capture session behind `fetch_viewport_rows`.
#[tauri::command]
pub fn capture_status(state: State<'_, AppCaptureState>) -> Result<CaptureStatus, String> {
    let engine = state.engine.read().map_err(|e| e.to_string())?;
    Ok(CaptureStatus {
        demo_mode: state.demo_mode,
        total_packets: engine.packet_count(),
        dropped_frames: engine.dropped_frames(),
    })
}

/// Serializes the visible capture window as the ICD §2 binary viewport batch.
///
/// The layout, clamping and payload walking are owned by
/// [`omniport_capture::viewport::ViewportBatch`]; this is only the Tauri adapter.
pub fn fetch_viewport_binary(
    engine: &MmapCaptureEngine,
    start_row: u64,
    row_count: usize,
) -> Vec<u8> {
    engine
        .read_viewport_batch(start_row, row_count)
        .encode_to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use omniport_core::{FrameFlags, PinState, SerialEvent};
    use tempfile::tempdir;

    #[test]
    fn test_fetch_viewport_binary_wire_format() {
        let dir = tempdir().unwrap();
        let mut engine = MmapCaptureEngine::create(dir.path(), 50, 4096).unwrap();

        // Append 3 packets
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

        // Fetch all 3 packets
        let wire = fetch_viewport_binary(&engine, 0, 10);
        assert!(wire.len() >= 16);

        // Header: [total: u64 (8B)][returned: u32 (4B)][reserved: u32 (4B)]
        let total = u64::from_le_bytes(wire[0..8].try_into().unwrap());
        let returned = u32::from_le_bytes(wire[8..12].try_into().unwrap());
        assert_eq!(total, 3);
        assert_eq!(returned, 3);

        // Inspect row 0
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

        // Test viewport slice: start_row = 1, row_count = 1
        let slice_wire = fetch_viewport_binary(&engine, 1, 1);
        let slice_total = u64::from_le_bytes(slice_wire[0..8].try_into().unwrap());
        let slice_returned = u32::from_le_bytes(slice_wire[8..12].try_into().unwrap());
        assert_eq!(slice_total, 3);
        assert_eq!(slice_returned, 1);

        let slice_ep_id = u16::from_le_bytes(slice_wire[16 + 20..16 + 22].try_into().unwrap());
        assert_eq!(slice_ep_id, 11); // Second packet
    }

    #[test]
    fn test_fetch_viewport_binary_out_of_bounds() {
        let dir = tempdir().unwrap();
        let engine = MmapCaptureEngine::create(dir.path(), 50, 4096).unwrap();

        let wire = fetch_viewport_binary(&engine, 100, 10);
        let total = u64::from_le_bytes(wire[0..8].try_into().unwrap());
        let returned = u32::from_le_bytes(wire[8..12].try_into().unwrap());
        assert_eq!(total, 0);
        assert_eq!(returned, 0);
    }

    #[test]
    fn test_fetch_viewport_binary_rejects_absurd_row_count_without_overallocating() {
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

        // Before the clamp this asked for `16 + usize::MAX * 96` bytes (overflow / abort).
        let wire = fetch_viewport_binary(&engine, 0, usize::MAX);
        let returned = u32::from_le_bytes(wire[8..12].try_into().unwrap());
        assert_eq!(returned, 3);

        // A wildly out-of-range start row must be handled without panicking either.
        let wire = fetch_viewport_binary(&engine, u64::MAX, usize::MAX);
        let returned = u32::from_le_bytes(wire[8..12].try_into().unwrap());
        assert_eq!(returned, 0);
    }

    #[test]
    fn test_fetch_viewport_binary_clamps_to_max_rows() {
        let dir = tempdir().unwrap();
        let max_packets = MAX_VIEWPORT_ROWS as u64 + 100;
        let mut engine = MmapCaptureEngine::create(dir.path(), max_packets, max_packets * 4).unwrap();
        for i in 0..max_packets {
            engine.append_event(&SerialEvent {
                timestamp_ns: i,
                source_id: 1,
                flags: FrameFlags::empty(),
                pins: PinState::empty(),
                payload: Bytes::from_static(b"AB"),
            });
        }

        let wire = fetch_viewport_binary(&engine, 0, usize::MAX);
        let total = u64::from_le_bytes(wire[0..8].try_into().unwrap());
        let returned = u32::from_le_bytes(wire[8..12].try_into().unwrap());
        assert_eq!(total, max_packets);
        assert_eq!(returned, MAX_VIEWPORT_ROWS as u32);
    }
}
