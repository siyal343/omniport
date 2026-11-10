//! OmniPort Desktop Tauri v2 Application Library.

pub mod commands;

pub use commands::{
    MAX_VIEWPORT_ROWS, CaptureStatus, fetch_viewport_binary, fetch_viewport_rows, capture_status,
    AppCaptureState,
};
use omniport_capture::MmapCaptureEngine;
use std::sync::RwLock;

/// Size of the desktop viewport ring: 100k frames / 16 MiB of payload data.
const DESKTOP_MAX_PACKETS: u64 = 100_000;
const DESKTOP_MAX_DATA_BYTES: u64 = 16 * 1024 * 1024;

/// Opens the dual-mmap capture store for the desktop viewport streamer.
///
/// Prefers the OS temp directory and falls back to `target/desktop_mmap` when
/// temp is unavailable (locked-down profiles, read-only temp, CI sandboxes).
fn create_capture_engine() -> std::io::Result<MmapCaptureEngine> {
    let capture_dir = std::env::temp_dir().join("omniport_desktop_capture");
    std::fs::create_dir_all(&capture_dir)?;
    match MmapCaptureEngine::create(&capture_dir, DESKTOP_MAX_PACKETS, DESKTOP_MAX_DATA_BYTES) {
        Ok(engine) => Ok(engine),
        Err(_primary_err) => {
            let fallback_dir = std::env::current_dir()?
                .join("target")
                .join("desktop_mmap");
            std::fs::create_dir_all(&fallback_dir)?;
            MmapCaptureEngine::create(&fallback_dir, DESKTOP_MAX_PACKETS, DESKTOP_MAX_DATA_BYTES)
        }
    }
}

/// Seeds a small deterministic frame set so the hex monitor, logic strip and
/// telemetry panels have content before `omniportd` is connected.
///
/// NOTE: this is a temporary bridge until the desktop client consumes the
/// daemon's live capture session; see docs/adr/ADR-001 (Daemon-First).
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

/// Decides whether the host fabricates a demo capture session at startup.
///
/// The seeded frames exist so the workbench shows something before `omniportd`
/// is connected. They must never masquerade as live traffic in a release build:
/// debug builds seed by default, release builds only when `OMNIPORT_DESKTOP_DEMO=1`,
/// and either can opt out with `OMNIPORT_DESKTOP_DEMO=0`.
fn demo_session_enabled() -> bool {
    match std::env::var("OMNIPORT_DESKTOP_DEMO").ok().as_deref() {
        Some("0") | Some("false") | Some("off") => false,
        Some(_) => true,
        None => cfg!(debug_assertions),
    }
}

/// Boots the Tauri workbench. Returns an error instead of panicking so the
/// binary can report a nonzero exit status to the OS on startup failure.
pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut engine = create_capture_engine()?;
    let demo_mode = demo_session_enabled();
    if demo_mode {
        seed_demo_session(&mut engine);
    }
    let app_state = AppCaptureState {
        engine: RwLock::new(engine),
        demo_mode,
    };

    let app = tauri::Builder::default()
        .manage(app_state)
        .invoke_handler(tauri::generate_handler![
            commands::fetch_viewport_rows,
            commands::capture_status
        ])
        .build(tauri::generate_context!())?;

    app.run(|_app_handle, _event| {});
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_capture_engine_succeeds_and_is_empty() {
        let engine = create_capture_engine().expect("capture engine should initialize");
        assert_eq!(engine.packet_count(), 0);
    }

    #[test]
    fn test_seed_demo_session_is_deterministic() {
        let dir = std::env::temp_dir().join(format!("omniport_seed_test_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let mut engine = MmapCaptureEngine::create(&dir, 64, 4096).unwrap();
        seed_demo_session(&mut engine);
        assert_eq!(engine.packet_count(), 25);
        let mut wire = Vec::new();
        engine.read_viewport(0, 1, &mut wire);
        assert!(wire.len() > 16);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
