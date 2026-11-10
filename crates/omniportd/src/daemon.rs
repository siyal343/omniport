use omniport_capture::MmapCaptureEngine;
use omniport_core::SerialEvent;
use omniport_core::routing::RoutingGraph;
use omniport_core::topology::{
    Request as TopologyRequest, Response as TopologyResponse, Topology, TopologyError,
};
use omniport_proto::{EndpointInfo, Request, Response};
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

pub struct DaemonEngine {
    start_time: Instant,
    topology: Mutex<Topology>,
    graph: Arc<RoutingGraph>,
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
        let topology = Topology::new();
        let graph = topology.graph();
        Self {
            start_time: Instant::now(),
            topology: Mutex::new(topology),
            graph,
            capture: RwLock::new(capture),
            shutdown_tx,
            is_shutting_down: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Returns the live `RoutingGraph` synchronized by the daemon's [`Topology`].
    pub fn routing_graph(&self) -> &Arc<RoutingGraph> {
        &self.graph
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
                Response::viewport_batch(bytes)
            }
            Request::CaptureStatus => {
                let capture = self.capture.read().await;
                Response::CaptureStatus {
                    total_packets: capture.engine.packet_count(),
                    dropped_frames: capture.engine.dropped_frames(),
                }
            }
            Request::Shutdown => {
                self.is_shutting_down.store(true, Ordering::SeqCst);
                let _ = self.shutdown_tx.send(());
                Response::Success {
                    message: "omniportd shutdown initiated".to_string(),
                }
            }
            Request::HealthCheck => {
                let (active_endpoints, active_routes) = self.topology.lock().await.counts();
                Response::Health {
                    status: "RUNNING".to_string(),
                    uptime_secs: self.start_time.elapsed().as_secs(),
                    active_endpoints: active_endpoints as u32,
                    active_routes: active_routes as u32,
                }
            }
            topo_req => {
                let mapped = match topo_req {
                    Request::ListEndpoints => TopologyRequest::ListEndpoints,
                    Request::GetTopology => TopologyRequest::Snapshot,
                    Request::CreateVirtualPair {
                        port_a,
                        port_b,
                        config,
                    } => TopologyRequest::CreateVirtualPair {
                        port_a,
                        port_b,
                        config,
                    },
                    Request::RemoveEndpoint { id } => TopologyRequest::RemoveEndpoint { id },
                    Request::AddRoute {
                        source_id,
                        target_ids,
                        bidirectional,
                        fault,
                    } => TopologyRequest::AddRoute {
                        source_id,
                        target_ids,
                        bidirectional,
                        fault,
                    },
                    Request::RemoveRoute { route_id } => TopologyRequest::RemoveRoute { route_id },
                    Request::HealthCheck
                    | Request::ReadViewport { .. }
                    | Request::CaptureStatus
                    | Request::Shutdown => unreachable!(),
                };

                let result = self.topology.lock().await.apply(mapped);
                map_topology_result(result)
            }
        }
    }
}

fn map_topology_result(result: Result<TopologyResponse, TopologyError>) -> Response {
    match result {
        Ok(TopologyResponse::Endpoints(eps)) => {
            Response::Endpoints(eps.into_iter().map(EndpointInfo::from).collect())
        }
        Ok(TopologyResponse::Snapshot(snapshot)) => Response::Topology {
            endpoints: snapshot
                .endpoints
                .into_iter()
                .map(EndpointInfo::from)
                .collect(),
            routes: snapshot.routes,
        },
        Ok(TopologyResponse::VirtualPairCreated {
            pair_id,
            endpoint_a,
            endpoint_b,
            ..
        }) => Response::VirtualPairCreated {
            pair_id,
            port_a: endpoint_a.name,
            port_b: endpoint_b.name,
            success: true,
            message: format!(
                "Virtual pair allocated (endpoints {} <-> {})",
                endpoint_a.id, endpoint_b.id
            ),
        },
        Ok(TopologyResponse::RouteAdded(route)) => Response::RouteAdded {
            route_id: route.route_id,
            success: true,
            message: format!("Route {} established", route.route_id),
        },
        Ok(TopologyResponse::Removed { message }) => Response::Success { message },
        Ok(other) => Response::Error {
            code: 500,
            message: format!("unexpected topology response: {other:?}"),
        },
        Err(err) => Response::Error {
            code: err.status_code(),
            message: err.to_string(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use omniport_core::{FrameFlags, PinState};
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
        match engine.handle_request(Request::HealthCheck).await {
            Response::Health {
                active_endpoints,
                active_routes,
                ..
            } => {
                assert_eq!(active_endpoints, 32_767 * 2);
                assert_eq!(active_routes, 32_767);
            }
            other => panic!("expected Health response, got {other:?}"),
        }
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
            Request::GetTopology,
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
                fault: None,
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
                fault: None,
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
                fault: None,
            })
            .await;
        assert!(matches!(resp, Response::Error { code: 400, .. }));

        // A valid route is still accepted and reflected in the live RoutingGraph.
        let resp = engine
            .handle_request(Request::AddRoute {
                source_id: 1,
                target_ids: vec![2],
                bidirectional: true,
                fault: None,
            })
            .await;
        assert!(matches!(resp, Response::RouteAdded { success: true, .. }));

        let evt = SerialEvent {
            timestamp_ns: 1_000,
            source_id: 1,
            flags: FrameFlags::DIR_TX,
            pins: PinState::empty(),
            payload: Bytes::from_static(b"TOPO_SYNC"),
        };
        assert_eq!(engine.routing_graph().route(evt).unwrap(), 1);
        let delivered = engine.routing_graph().recv(2).await.unwrap();
        assert_eq!(&delivered.payload[..], b"TOPO_SYNC");
    }
}
