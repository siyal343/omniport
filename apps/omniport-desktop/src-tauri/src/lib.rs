//! OmniPort Desktop Tauri v2 Application Library.

pub mod commands;

pub use commands::{fetch_viewport_binary, fetch_viewport_rows, AppCaptureState};
use omniport_capture::MmapCaptureEngine;
use std::sync::RwLock;

pub fn run() {
    // 1. Initialize dual-mmap capture engine for desktop viewport streaming
    let capture_dir = std::env::temp_dir().join("omniport_desktop_capture");
    let _ = std::fs::create_dir_all(&capture_dir);

    let mut engine = MmapCaptureEngine::create(&capture_dir, 100_000, 16 * 1024 * 1024)
        .unwrap_or_else(|_| {
            let fallback_dir = std::env::current_dir()
                .unwrap_or_else(|_| std::path::PathBuf::from("."))
                .join("target")
                .join("desktop_mmap");
            let _ = std::fs::create_dir_all(&fallback_dir);
            MmapCaptureEngine::create(&fallback_dir, 100_000, 16 * 1024 * 1024)
                .expect("failed to initialize desktop capture store")
        });

    // Seed baseline telemetry frames so hex monitor & logic strip render immediately
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

    let app_state = AppCaptureState(RwLock::new(engine));

    // 2. Launch Tauri workbench application
    tauri::Builder::default()
        .manage(app_state)
        .invoke_handler(tauri::generate_handler![commands::fetch_viewport_rows])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
