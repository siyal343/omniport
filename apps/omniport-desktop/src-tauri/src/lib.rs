//! OmniPort Desktop Tauri v2 Application Library.

pub mod capture_store;
pub mod commands;

pub use capture_store::{
    CaptureStatus, CaptureStore, DEFAULT_DAEMON_ADDR, DaemonCaptureStore, DisconnectedCaptureStore,
    FixtureCaptureStore, StoreFuture,
};
pub use commands::{AppCaptureState, MAX_VIEWPORT_ROWS, capture_status, fetch_viewport_rows};
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Eq)]
enum CaptureStoreTarget {
    Disconnected(String),
    FixtureDemo,
    Daemon { addr: String },
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

fn resolve_daemon_addr(var: Option<&str>) -> String {
    match var.map(str::trim) {
        Some(addr) if !addr.is_empty() => addr.to_string(),
        _ => DEFAULT_DAEMON_ADDR.to_string(),
    }
}

/// Selects the single `CaptureStore` adapter once at host startup.
///
/// Shipping release builds default to `DaemonCaptureStore` (`omniportd`). The
/// demo fixture adapter is only selected for debug builds (when no daemon
/// address was specified) or when `OMNIPORT_DESKTOP_DEMO=1` is explicitly set.
fn select_capture_target(
    disconnect_env: Option<&str>,
    demo_env: Option<&str>,
    daemon_addr_env: Option<&str>,
    is_debug: bool,
) -> CaptureStoreTarget {
    if let Some(reason) = forced_disconnect_reason(disconnect_env) {
        return CaptureStoreTarget::Disconnected(reason);
    }

    let addr = resolve_daemon_addr(daemon_addr_env);
    let has_explicit_addr = daemon_addr_env.is_some_and(|s| !s.trim().is_empty());

    match demo_env.map(str::trim) {
        Some("0") | Some("false") | Some("off") => CaptureStoreTarget::Daemon { addr },
        Some("") | None => {
            if has_explicit_addr || !is_debug {
                CaptureStoreTarget::Daemon { addr }
            } else {
                CaptureStoreTarget::FixtureDemo
            }
        }
        Some(_) => CaptureStoreTarget::FixtureDemo,
    }
}

fn build_capture_state() -> AppCaptureState {
    let target = select_capture_target(
        std::env::var("OMNIPORT_DESKTOP_DISCONNECT").ok().as_deref(),
        std::env::var("OMNIPORT_DESKTOP_DEMO").ok().as_deref(),
        std::env::var("OMNIPORT_DAEMON_ADDR").ok().as_deref(),
        cfg!(debug_assertions),
    );

    match target {
        CaptureStoreTarget::Disconnected(reason) => AppCaptureState::disconnected(reason),
        CaptureStoreTarget::Daemon { addr } => {
            AppCaptureState::new(Arc::new(DaemonCaptureStore::new(addr)))
        }
        CaptureStoreTarget::FixtureDemo => match FixtureCaptureStore::seeded() {
            Ok(fixture) => AppCaptureState::new(Arc::new(fixture)),
            Err(err) => {
                AppCaptureState::disconnected(format!("Failed to open demo fixture store: {err}"))
            }
        },
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
    fn test_select_capture_target_respects_profile_and_env() {
        // Release build defaults to DaemonCaptureStore.
        assert_eq!(
            select_capture_target(None, None, None, false),
            CaptureStoreTarget::Daemon {
                addr: DEFAULT_DAEMON_ADDR.to_string()
            }
        );
        // Debug build defaults to FixtureCaptureStore unless daemon addr or DEMO=0 is set.
        assert_eq!(
            select_capture_target(None, None, None, true),
            CaptureStoreTarget::FixtureDemo
        );
        assert_eq!(
            select_capture_target(None, None, Some("127.0.0.1:6000"), true),
            CaptureStoreTarget::Daemon {
                addr: "127.0.0.1:6000".to_string()
            }
        );
        assert_eq!(
            select_capture_target(None, Some("0"), None, true),
            CaptureStoreTarget::Daemon {
                addr: DEFAULT_DAEMON_ADDR.to_string()
            }
        );
        assert_eq!(
            select_capture_target(None, Some("false"), None, true),
            CaptureStoreTarget::Daemon {
                addr: DEFAULT_DAEMON_ADDR.to_string()
            }
        );
        // Release build still allows explicit demo opt-in.
        assert_eq!(
            select_capture_target(None, Some("1"), None, false),
            CaptureStoreTarget::FixtureDemo
        );
        assert_eq!(
            select_capture_target(None, Some(""), None, false),
            CaptureStoreTarget::Daemon {
                addr: DEFAULT_DAEMON_ADDR.to_string()
            }
        );
        // Disconnect override wins over everything.
        assert!(matches!(
            select_capture_target(Some("1"), Some("1"), None, true),
            CaptureStoreTarget::Disconnected(_)
        ));
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
