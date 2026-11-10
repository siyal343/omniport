//! OmniPort Desktop Tauri v2 Application Library.

pub mod capture_store;
pub mod commands;
pub mod topology_commands;
pub mod topology_store;

pub use capture_store::{
    CaptureStatus, CaptureStore, DEFAULT_DAEMON_ADDR, DaemonCaptureStore, DisconnectedCaptureStore,
    FixtureCaptureStore, StoreFuture,
};
pub use commands::{AppCaptureState, MAX_VIEWPORT_ROWS, capture_status, fetch_viewport_rows};
use std::sync::Arc;
pub use topology_commands::{
    AppTopologyState, add_route, create_virtual_pair, get_topology, remove_endpoint, remove_route,
};
pub use topology_store::{
    AddRouteRequest, CreatePairRequest, DaemonTopology, DisconnectedTopology, FixtureTopology,
    TopologyControl, TopologyView,
};

#[derive(Debug, Clone, PartialEq, Eq)]
enum HostTarget {
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

/// Selects the single backend target once at host startup.
///
/// Shipping release builds default to the live daemon adapters (`omniportd`).
/// The demo fixtures are only selected for debug builds (when no daemon address
/// was specified) or when `OMNIPORT_DESKTOP_DEMO=1` is explicitly set.
fn select_host_target(
    disconnect_env: Option<&str>,
    demo_env: Option<&str>,
    daemon_addr_env: Option<&str>,
    is_debug: bool,
) -> HostTarget {
    if let Some(reason) = forced_disconnect_reason(disconnect_env) {
        return HostTarget::Disconnected(reason);
    }

    let addr = resolve_daemon_addr(daemon_addr_env);
    let has_explicit_addr = daemon_addr_env.is_some_and(|s| !s.trim().is_empty());

    match demo_env.map(str::trim) {
        Some("0") | Some("false") | Some("off") => HostTarget::Daemon { addr },
        Some("") | None => {
            if has_explicit_addr || !is_debug {
                HostTarget::Daemon { addr }
            } else {
                HostTarget::FixtureDemo
            }
        }
        Some(_) => HostTarget::FixtureDemo,
    }
}

fn build_capture_state(target: &HostTarget) -> AppCaptureState {
    match target {
        HostTarget::Disconnected(reason) => AppCaptureState::disconnected(reason.clone()),
        HostTarget::Daemon { addr } => {
            AppCaptureState::new(Arc::new(DaemonCaptureStore::new(addr.clone())))
        }
        HostTarget::FixtureDemo => match FixtureCaptureStore::seeded() {
            Ok(fixture) => AppCaptureState::new(Arc::new(fixture)),
            Err(err) => {
                AppCaptureState::disconnected(format!("Failed to open demo fixture store: {err}"))
            }
        },
    }
}

fn build_topology_state(target: &HostTarget) -> AppTopologyState {
    match target {
        HostTarget::Disconnected(reason) => AppTopologyState::disconnected(reason.clone()),
        HostTarget::Daemon { addr } => {
            AppTopologyState::new(Arc::new(DaemonTopology::new(addr.clone())))
        }
        HostTarget::FixtureDemo => match FixtureTopology::seeded() {
            Ok(topology) => AppTopologyState::new(Arc::new(topology)),
            Err(err) => {
                AppTopologyState::disconnected(format!("Failed to open demo topology: {err}"))
            }
        },
    }
}

/// Boots the Tauri workbench. Returns an error instead of panicking so the
/// binary can report a nonzero exit status to the OS on startup failure.
pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    let target = select_host_target(
        std::env::var("OMNIPORT_DESKTOP_DISCONNECT").ok().as_deref(),
        std::env::var("OMNIPORT_DESKTOP_DEMO").ok().as_deref(),
        std::env::var("OMNIPORT_DAEMON_ADDR").ok().as_deref(),
        cfg!(debug_assertions),
    );

    let app = tauri::Builder::default()
        .manage(build_capture_state(&target))
        .manage(build_topology_state(&target))
        .invoke_handler(tauri::generate_handler![
            commands::fetch_viewport_rows,
            commands::capture_status,
            topology_commands::get_topology,
            topology_commands::create_virtual_pair,
            topology_commands::add_route,
            topology_commands::remove_route,
            topology_commands::remove_endpoint
        ])
        .build(tauri::generate_context!())?;

    app.run(|_app_handle, _event| {});
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_select_host_target_respects_profile_and_env() {
        // Release build defaults to the daemon adapters.
        assert_eq!(
            select_host_target(None, None, None, false),
            HostTarget::Daemon {
                addr: DEFAULT_DAEMON_ADDR.to_string()
            }
        );
        // Debug build defaults to the demo fixtures unless an addr or DEMO=0 is set.
        assert_eq!(
            select_host_target(None, None, None, true),
            HostTarget::FixtureDemo
        );
        assert_eq!(
            select_host_target(None, None, Some("127.0.0.1:6000"), true),
            HostTarget::Daemon {
                addr: "127.0.0.1:6000".to_string()
            }
        );
        assert_eq!(
            select_host_target(None, Some("0"), None, true),
            HostTarget::Daemon {
                addr: DEFAULT_DAEMON_ADDR.to_string()
            }
        );
        assert_eq!(
            select_host_target(None, Some("false"), None, true),
            HostTarget::Daemon {
                addr: DEFAULT_DAEMON_ADDR.to_string()
            }
        );
        // Release build still allows explicit demo opt-in.
        assert_eq!(
            select_host_target(None, Some("1"), None, false),
            HostTarget::FixtureDemo
        );
        assert_eq!(
            select_host_target(None, Some(""), None, false),
            HostTarget::Daemon {
                addr: DEFAULT_DAEMON_ADDR.to_string()
            }
        );
        // Disconnect override wins over everything.
        assert!(matches!(
            select_host_target(Some("1"), Some("1"), None, true),
            HostTarget::Disconnected(_)
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

    #[tokio::test]
    async fn test_fixture_target_builds_interactive_topology() {
        let state = build_topology_state(&HostTarget::FixtureDemo);
        let view = state.snapshot().await.expect("fixture topology snapshot");
        assert_eq!(view.endpoints.len(), 3);
        assert_eq!(view.routes.len(), 2);
    }

    #[tokio::test]
    async fn test_disconnected_target_surfaces_reason_in_both_states() {
        let target = HostTarget::Disconnected("host unavailable".to_string());
        let capture = build_capture_state(&target);
        let topology = build_topology_state(&target);
        assert_eq!(
            capture.status().await.unwrap_err(),
            "host unavailable",
            "capture state must surface the disconnect reason"
        );
        assert_eq!(
            topology.snapshot().await.unwrap_err(),
            "host unavailable",
            "topology state must surface the disconnect reason"
        );
    }
}
