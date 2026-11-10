// Prevents additional console window on Windows in release
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    if let Err(err) = omniport_desktop_lib::run() {
        eprintln!("omniport-desktop failed to start: {err}");
        std::process::exit(1);
    }
}
