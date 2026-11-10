//! OmniPort Desktop Tauri v2 Application Library.

pub mod commands;

pub use commands::{
    AppCaptureState, CaptureStatus, MAX_VIEWPORT_ROWS, capture_status, fetch_viewport_binary,
    fetch_viewport_rows,
};
use omniport_capture::MmapCaptureEngine;

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
            let fallback_dir = std::env::current_dir()?.join("target").join("desktop_mmap");
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
fn demo_session_enabled_from(var: Option<&str>, is_debug: bool) -> bool {
    match var {
        Some("0") | Some("false") | Some("off") => false,
        Some(_) => true,
        None => is_debug,
    }
}

fn demo_session_enabled() -> bool {
    demo_session_enabled_from(
        std::env::var("OMNIPORT_DESKTOP_DEMO").ok().as_deref(),
        cfg!(debug_assertions),
    )
}

/// Optional fault-injection hook (`OMNIPORT_DESKTOP_DISCONNECT=1`) so smoke
/// tests can verify the WebView's `CAPTURE DISCONNECTED` banner against a real
/// failed Tauri host IPC response.
fn forced_disconnect_reason(var: Option<&str>) -> Option<String> {
    match var {
        None | Some("") | Some("0") | Some("false") | Some("off") => None,
        Some("1") | Some("true") | Some("on") => {
            Some("Host capture store disconnected (OMNIPORT_DESKTOP_DISCONNECT)".to_string())
        }
        Some(custom) => Some(custom.to_string()),
    }
}

fn build_capture_state() -> AppCaptureState {
    if let Some(reason) =
        forced_disconnect_reason(std::env::var("OMNIPORT_DESKTOP_DISCONNECT").ok().as_deref())
    {
        return AppCaptureState::disconnected(reason);
    }

    match create_capture_engine() {
        Ok(mut engine) => {
            let demo_mode = demo_session_enabled();
            if demo_mode {
                seed_demo_session(&mut engine);
            }
            AppCaptureState::connected(engine, demo_mode)
        }
        Err(err) => {
            AppCaptureState::disconnected(format!("Failed to open desktop capture store: {err}"))
        }
    }
}

/// Boots the Tauri workbench. Returns an error instead of panicking so the
/// binary can report a nonzero exit status to the OS on startup failure.
pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    let app_state = build_capture_state();

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

    #[test]
    fn test_demo_session_enabled_respects_profile_and_env() {
        assert!(!demo_session_enabled_from(None, false));
        assert!(demo_session_enabled_from(None, true));
        assert!(!demo_session_enabled_from(Some("0"), true));
        assert!(!demo_session_enabled_from(Some("false"), true));
        assert!(!demo_session_enabled_from(Some("off"), true));
        assert!(demo_session_enabled_from(Some("1"), false));
        assert!(demo_session_enabled_from(Some("true"), false));
    }

    #[test]
    fn test_forced_disconnect_reason_parses_env() {
        assert_eq!(forced_disconnect_reason(None), None);
        assert_eq!(forced_disconnect_reason(Some("")), None);
        assert_eq!(forced_disconnect_reason(Some("0")), None);
        assert_eq!(forced_disconnect_reason(Some("false")), None);
        assert!(forced_disconnect_reason(Some("1")).is_some());
        assert_eq!(
            forced_disconnect_reason(Some("custom fault")).as_deref(),
            Some("custom fault")
        );
    }
}
