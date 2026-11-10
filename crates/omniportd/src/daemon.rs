use omniport_capture::MmapCaptureEngine;
use omniport_core::SerialEvent;
use omniport_proto::{EndpointInfo, Request, Response};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;
use tokio::sync::{Mutex, RwLock, broadcast};

const DAEMON_CAPTURE_MAX_PACKETS: u64 = 100_000;
const DAEMON_CAPTURE_MAX_BYTES: u64 = 16 * 1024 * 1024;

struct DaemonCapture {
    // Drop `engine` before `_dir` so memory-mapped files unmap before TempDir cleanup on Windows.
    engine: MmapCaptureEngine,
    _dir: Option<tempfile::TempDir>,
}

#[derive(Debug, Clone)]
pub struct RouteEntry {
    pub route_id: u32,
    pub source_id: u16,
    pub target_ids: Vec<u16>,
    pub bidirectional: bool,
}

#[derive(Debug)]
pub struct DaemonState {
    pub start_time: Instant,
    pub endpoints: HashMap<u16, EndpointInfo>,
    pub routes: HashMap<u32, RouteEntry>,
    pub next_endpoint_id: u16,
    pub next_route_id: u32,
    pub next_pair_id: u32,
}

impl Default for DaemonState {
    fn default() -> Self {
        Self {
            start_time: Instant::now(),
            endpoints: HashMap::new(),
            routes: HashMap::new(),
            next_endpoint_id: 1,
            next_route_id: 100,
            next_pair_id: 1,
        }
    }
}

pub struct DaemonEngine {
    pub state: Arc<Mutex<DaemonState>>,
    capture: RwLock<DaemonCapture>,
    pub shutdown_tx: broadcast::Sender<()>,
    pub is_shutting_down: Arc<AtomicBool>,
}

impl Default for DaemonEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl DaemonEngine {
    pub fn new() -> Self {
        Self::try_new().expect("failed to initialize omniportd capture store")
    }

    pub fn try_new() -> std::io::Result<Self> {
        let dir = tempfile::tempdir()?;
        let engine = MmapCaptureEngine::create(
            dir.path(),
            DAEMON_CAPTURE_MAX_PACKETS,
            DAEMON_CAPTURE_MAX_BYTES,
        )?;
        Ok(Self::from_capture(DaemonCapture {
            engine,
            _dir: Some(dir),
        }))
    }

    pub fn with_capture_dir(dir: &Path) -> std::io::Result<Self> {
        std::fs::create_dir_all(dir)?;
        let engine =
            MmapCaptureEngine::create(dir, DAEMON_CAPTURE_MAX_PACKETS, DAEMON_CAPTURE_MAX_BYTES)?;
        Ok(Self::from_capture(DaemonCapture { engine, _dir: None }))
    }

    fn from_capture(capture: DaemonCapture) -> Self {
        let (shutdown_tx, _) = broadcast::channel(4);
        Self {
            state: Arc::new(Mutex::new(DaemonState::default())),
            capture: RwLock::new(capture),
            shutdown_tx,
            is_shutting_down: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Appends a captured serial frame to the daemon's dual-mmap store.
    pub async fn record_event(&self, event: &SerialEvent) {
        self.capture.write().await.engine.append_event(event);
    }

    pub fn subscribe_shutdown(&self) -> broadcast::Receiver<()> {
        self.shutdown_tx.subscribe()
    }

    pub async fn handle_request(&self, req: Request) -> Response {
        // Once shutdown has been requested, stop admitting new control-plane
        // work (the flag used to be written but never read).
        if self.is_shutting_down.load(Ordering::SeqCst) && !matches!(req, Request::Shutdown) {
            return Response::Error {
                code: 503,
                message: "omniportd is shutting down".to_string(),
            };
        }

        match req {
            Request::ReadViewport {
                start_row,
                row_count,
            } => {
                let capture = self.capture.read().await;
                let bytes = capture
                    .engine
                    .read_viewport_batch(start_row, row_count as usize)
                    .encode_to_vec();
                return Response::ViewportBatch { bytes };
            }
            Request::CaptureStatus => {
                let capture = self.capture.read().await;
                return Response::CaptureStatus {
                    total_packets: capture.engine.packet_count(),
                    dropped_frames: capture.engine.dropped_frames(),
                };
            }
            _ => {}
        }

        let mut state = self.state.lock().await;

        match req {
            Request::HealthCheck => {
                let uptime_secs = state.start_time.elapsed().as_secs();
                let active_endpoints = state.endpoints.len() as u32;
                let active_routes = state.routes.len() as u32;
                Response::Health {
                    status: "RUNNING".to_string(),
                    uptime_secs,
                    active_endpoints,
                    active_routes,
                }
            }

            Request::ListEndpoints => {
                let list: Vec<EndpointInfo> = state.endpoints.values().cloned().collect();
                Response::Endpoints(list)
            }

            Request::CreateVirtualPair {
                port_a,
                port_b,
                config,
            } => {
                // A u16 endpoint id space is only 65535 wide: reject instead of
                // overflowing (debug panic / release wrap) after enough pairs.
                let id_a = state.next_endpoint_id;
                let Some(id_b) = id_a.checked_add(1) else {
                    return Response::Error {
                        code: 503,
                        message: "endpoint id space exhausted".to_string(),
                    };
                };
                let Some(next_endpoint_id) = id_b.checked_add(1) else {
                    return Response::Error {
                        code: 503,
                        message: "endpoint id space exhausted".to_string(),
                    };
                };
                let Some(next_pair_id) = state.next_pair_id.checked_add(1) else {
                    return Response::Error {
                        code: 503,
                        message: "virtual pair id space exhausted".to_string(),
                    };
                };
                let Some(next_route_id) = state.next_route_id.checked_add(1) else {
                    return Response::Error {
                        code: 503,
                        message: "route id space exhausted".to_string(),
                    };
                };

                let pair_id = state.next_pair_id;
                state.next_pair_id = next_pair_id;
                state.next_endpoint_id = next_endpoint_id;

                state.endpoints.insert(
                    id_a,
                    EndpointInfo {
                        id: id_a,
                        name: port_a.clone(),
                        port_type: "VirtualPairSideA".to_string(),
                        baud_rate: config.baud_rate,
                        is_active: true,
                    },
                );

                state.endpoints.insert(
                    id_b,
                    EndpointInfo {
                        id: id_b,
                        name: port_b.clone(),
                        port_type: "VirtualPairSideB".to_string(),
                        baud_rate: config.baud_rate,
                        is_active: true,
                    },
                );

                // Add default bidirectional 1:1 route between the pair
                let route_id = state.next_route_id;
                state.next_route_id = next_route_id;
                state.routes.insert(
                    route_id,
                    RouteEntry {
                        route_id,
                        source_id: id_a,
                        target_ids: vec![id_b],
                        bidirectional: true,
                    },
                );

                Response::VirtualPairCreated {
                    pair_id,
                    port_a,
                    port_b,
                    success: true,
                    message: format!("Virtual pair allocated (endpoints {} <-> {})", id_a, id_b),
                }
            }

            Request::RemoveEndpoint { id } => {
                if state.endpoints.remove(&id).is_some() {
                    // Remove connected routes
                    state
                        .routes
                        .retain(|_, r| r.source_id != id && !r.target_ids.contains(&id));
                    Response::Success {
                        message: format!("Endpoint {} removed successfully", id),
                    }
                } else {
                    Response::Error {
                        code: 404,
                        message: format!("Endpoint {} not found", id),
                    }
                }
            }

            Request::AddRoute {
                source_id,
                target_ids,
                bidirectional,
            } => {
                // A route referencing nonexistent endpoints could never deliver
                // frames; report that instead of claiming success.
                if !state.endpoints.contains_key(&source_id) {
                    return Response::Error {
                        code: 404,
                        message: format!("Endpoint {} not found", source_id),
                    };
                }
                if target_ids.is_empty() {
                    return Response::Error {
                        code: 400,
                        message: "AddRoute requires at least one target endpoint".to_string(),
                    };
                }
                if let Some(missing) = target_ids
                    .iter()
                    .find(|id| !state.endpoints.contains_key(id))
                {
                    return Response::Error {
                        code: 404,
                        message: format!("Endpoint {} not found", missing),
                    };
                }

                let Some(next_route_id) = state.next_route_id.checked_add(1) else {
                    return Response::Error {
                        code: 503,
                        message: "route id space exhausted".to_string(),
                    };
                };
                let route_id = state.next_route_id;
                state.next_route_id = next_route_id;
                state.routes.insert(
                    route_id,
                    RouteEntry {
                        route_id,
                        source_id,
                        target_ids,
                        bidirectional,
                    },
                );
                Response::RouteAdded {
                    route_id,
                    success: true,
                    message: format!("Route {} established", route_id),
                }
            }

            Request::RemoveRoute { route_id } => {
                if state.routes.remove(&route_id).is_some() {
                    Response::Success {
                        message: format!("Route {} removed successfully", route_id),
                    }
                } else {
                    Response::Error {
                        code: 404,
                        message: format!("Route {} not found", route_id),
                    }
                }
            }

            Request::Shutdown => {
                self.is_shutting_down.store(true, Ordering::SeqCst);
                let _ = self.shutdown_tx.send(());
                Response::Success {
                    message: "omniportd shutdown initiated".to_string(),
                }
            }
            Request::ReadViewport { .. } | Request::CaptureStatus => {
                unreachable!("capture queries are handled before locking daemon state")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use omniport_proto::SerialConfigDto;

    fn pair_request(a: &str, b: &str) -> Request {
        Request::CreateVirtualPair {
            port_a: a.to_string(),
            port_b: b.to_string(),
            config: SerialConfigDto::default(),
        }
    }

    #[tokio::test]
    async fn test_endpoint_id_exhaustion_returns_error_instead_of_panicking() {
        let engine = DaemonEngine::new();

        // Each pair consumes two u16 ids: 32767 pairs exhaust ids 1..=65534.
        for i in 0..32_767 {
            let a = format!("COM{}", i);
            let b = format!("COM{}", i + 1);
            let resp = engine.handle_request(pair_request(&a, &b)).await;
            assert!(
                matches!(resp, Response::VirtualPairCreated { .. }),
                "unexpected response while filling id space: {:?}",
                resp
            );
        }

        // This request would overflow u16; it must be rejected, not panic.
        let resp = engine.handle_request(pair_request("COMX", "COMY")).await;
        match resp {
            Response::Error { code, message } => {
                assert_eq!(code, 503);
                assert!(
                    message.contains("exhausted"),
                    "unexpected message: {message}"
                );
            }
            other => panic!("expected 503 error on id exhaustion, got {other:?}"),
        }

        // No endpoint may have been registered for the rejected pair.
        let state = engine.state.lock().await;
        assert_eq!(state.endpoints.len(), 32_767 * 2);
    }

    #[tokio::test]
    async fn test_requests_are_rejected_during_shutdown() {
        let engine = DaemonEngine::new();
        let resp = engine.handle_request(Request::Shutdown).await;
        assert!(matches!(resp, Response::Success { .. }));
        assert!(engine.is_shutting_down.load(Ordering::SeqCst));

        // Health checks, capture queries, and mutations must not be served after shutdown.
        for req in [
            Request::HealthCheck,
            Request::ListEndpoints,
            Request::CaptureStatus,
            Request::ReadViewport {
                start_row: 0,
                row_count: 10,
            },
        ] {
            let resp = engine.handle_request(req).await;
            assert!(
                matches!(resp, Response::Error { code: 503, .. }),
                "expected 503 during shutdown, got {resp:?}"
            );
        }

        // A repeated Shutdown is idempotent (still success).
        let resp = engine.handle_request(Request::Shutdown).await;
        assert!(matches!(resp, Response::Success { .. }));
    }

    #[tokio::test]
    async fn test_add_route_rejects_unknown_endpoints() {
        let engine = DaemonEngine::new();

        let resp = engine
            .handle_request(Request::AddRoute {
                source_id: 999,
                target_ids: vec![2],
                bidirectional: false,
            })
            .await;
        match resp {
            Response::Error { code, message } => {
                assert_eq!(code, 404);
                assert!(message.contains("999"), "unexpected message: {message}");
            }
            other => panic!("expected 404 for unknown source, got {other:?}"),
        }

        // Create one real endpoint, then reference a missing target.
        engine.handle_request(pair_request("COM1", "COM2")).await;
        let resp = engine
            .handle_request(Request::AddRoute {
                source_id: 1,
                target_ids: vec![2, 4242],
                bidirectional: false,
            })
            .await;
        match resp {
            Response::Error { code, message } => {
                assert_eq!(code, 404);
                assert!(message.contains("4242"), "unexpected message: {message}");
            }
            other => panic!("expected 404 for unknown target, got {other:?}"),
        }

        // Empty target list is a client error, not a silent no-op route.
        let resp = engine
            .handle_request(Request::AddRoute {
                source_id: 1,
                target_ids: vec![],
                bidirectional: false,
            })
            .await;
        assert!(matches!(resp, Response::Error { code: 400, .. }));

        // A valid route is still accepted.
        let resp = engine
            .handle_request(Request::AddRoute {
                source_id: 1,
                target_ids: vec![2],
                bidirectional: true,
            })
            .await;
        assert!(matches!(resp, Response::RouteAdded { success: true, .. }));
    }
}
