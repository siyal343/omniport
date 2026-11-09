use omniport_proto::{EndpointInfo, Request, Response};
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;
use tokio::sync::{Mutex, broadcast};

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
        let (shutdown_tx, _) = broadcast::channel(4);
        Self {
            state: Arc::new(Mutex::new(DaemonState::default())),
            shutdown_tx,
            is_shutting_down: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn subscribe_shutdown(&self) -> broadcast::Receiver<()> {
        self.shutdown_tx.subscribe()
    }

    pub async fn handle_request(&self, req: Request) -> Response {
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
                let pair_id = state.next_pair_id;
                state.next_pair_id += 1;

                let id_a = state.next_endpoint_id;
                state.next_endpoint_id += 1;
                let id_b = state.next_endpoint_id;
                state.next_endpoint_id += 1;

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
                state.next_route_id += 1;
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
                let route_id = state.next_route_id;
                state.next_route_id += 1;
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
        }
    }
}
