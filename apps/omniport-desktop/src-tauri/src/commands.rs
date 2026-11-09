use omniport_capture::MmapCaptureEngine;
use std::sync::RwLock;
use tauri::{ipc::Response, State};

pub struct AppCaptureState(pub RwLock<MmapCaptureEngine>);

#[tauri::command]
pub fn fetch_viewport_rows(
    state: State<'_, AppCaptureState>,
    start_row: u64,
    row_count: usize,
) -> Result<Response, String> {
    let engine = state.0.read().map_err(|e| e.to_string())?;
    let mut raw_wire_buf = Vec::with_capacity(16 + row_count * 96);
    engine.read_viewport(start_row, row_count, &mut raw_wire_buf);
    Ok(Response::new(raw_wire_buf))
}
