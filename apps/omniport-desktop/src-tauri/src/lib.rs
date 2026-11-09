//! OmniPort Desktop Tauri v2 Application Library.

pub mod commands;

pub use commands::{fetch_viewport_binary, fetch_viewport_rows, AppCaptureState};

pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![commands::fetch_viewport_rows,])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
