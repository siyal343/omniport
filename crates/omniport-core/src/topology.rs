//! Unified topology lifecycle and route model (`Topology::apply`).
//!
//! Owns endpoint registration, virtual-pair/route ID allocation, live-endpoint
//! validation, and reconciliation into [`RoutingGraph`] directed edges so the
//! control plane (`omniportd`), declarative CI scenarios (`omniport-cli`), and
//! the data plane cannot diverge on route shape or lifecycle state.

use crate::fault::FaultConfig;
use crate::routing::{BackpressurePolicy, RoutingError, RoutingGraph};
use crate::{EndpointId, SerialConfig};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::sync::Arc;
use thiserror::Error;

pub const DEFAULT_QUEUE_CAPACITY: usize = 1024;

type EdgeKey = (EndpointId, EndpointId);
type PlannedEdge = (EdgeKey, Option<FaultConfig>);

/// Canonical endpoint metadata held inside [`Topology`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EndpointInfo {
    pub id: EndpointId,
    pub name: String,
    pub port_type: String,
    pub config: SerialConfig,
    pub is_active: bool,
}

/// Canonical route rule across `omniportd`, scenarios, and the desktop UI.
///
/// Expands into one or more [`DirectedEdge`] entries inside [`RoutingGraph`]
/// (fan-out across `target_ids`, plus reverse edges when `bidirectional` is set).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Route {
    pub route_id: u32,
    pub source_id: EndpointId,
    pub target_ids: Vec<EndpointId>,
    pub bidirectional: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fault: Option<FaultConfig>,
}

impl Route {
    /// Expands this route into its directed `(source -> target)` edges.
    pub fn directed_edges(&self) -> Vec<DirectedEdge> {
        let mut edges =
            Vec::with_capacity(self.target_ids.len() * if self.bidirectional { 2 } else { 1 });
        for &target_id in &self.target_ids {
            edges.push(DirectedEdge {
                route_id: self.route_id,
                source_id: self.source_id,
                target_id,
                bidirectional: self.bidirectional,
                fault: self.fault.clone(),
            });
            if self.bidirectional {
                edges.push(DirectedEdge {
                    route_id: self.route_id,
                    source_id: target_id,
                    target_id: self.source_id,
                    bidirectional: true,
                    fault: self.fault.clone(),
                });
            }
        }
        edges
    }
}

/// One directed edge projected from a [`Route`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DirectedEdge {
    pub route_id: u32,
    pub source_id: EndpointId,
    pub target_id: EndpointId,
    pub bidirectional: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fault: Option<FaultConfig>,
}

/// Consistent point-in-time view of all endpoints and routes in a [`Topology`].
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct TopologySnapshot {
    pub endpoints: Vec<EndpointInfo>,
    pub routes: Vec<Route>,
}

/// Commands and queries handled by [`Topology::apply`].
#[derive(Debug, Clone, PartialEq)]
pub enum Request {
    ListEndpoints,
    ListRoutes,
    Snapshot,
    RegisterEndpoint {
        /// Explicit ID for declarative scenarios, or `None` to allocate behind the seam.
        id: Option<EndpointId>,
        name: String,
        port_type: String,
        config: SerialConfig,
        queue_capacity: usize,
        backpressure: BackpressurePolicy,
    },
    CreateVirtualPair {
        port_a: String,
        port_b: String,
        config: SerialConfig,
    },
    RemoveEndpoint {
        id: EndpointId,
    },
    AddRoute {
        source_id: EndpointId,
        target_ids: Vec<EndpointId>,
        bidirectional: bool,
        fault: Option<FaultConfig>,
    },
    RemoveRoute {
        route_id: u32,
    },
}

/// Responses returned by [`Topology::apply`].
#[derive(Debug, Clone, PartialEq)]
pub enum Response {
    Endpoints(Vec<EndpointInfo>),
    Routes(Vec<Route>),
    Snapshot(TopologySnapshot),
    EndpointRegistered(EndpointInfo),
    VirtualPairCreated {
        pair_id: u32,
        endpoint_a: EndpointInfo,
        endpoint_b: EndpointInfo,
        route: Route,
    },
    RouteAdded(Route),
    Removed {
        message: String,
    },
}

#[derive(Error, Debug, PartialEq)]
pub enum TopologyError {
    #[error("Endpoint {0} not found")]
    EndpointNotFound(EndpointId),
    #[error("Endpoint {0} already registered")]
    EndpointAlreadyExists(EndpointId),
    #[error("Route {0} not found")]
    RouteNotFound(u32),
    #[error("Endpoint name cannot be empty")]
    EmptyEndpointName,
    #[error("AddRoute requires at least one target endpoint")]
    EmptyTargets,
    #[error("Route cannot target source endpoint {0} directly")]
    SelfLoop(EndpointId),
    #[error("Duplicate target endpoint {0} in route")]
    DuplicateTarget(EndpointId),
    #[error("Invalid fault configuration: {0}")]
    InvalidFaultConfig(String),
    #[error("Conflicting fault configuration on edge {from} -> {to}")]
    ConflictingEdgeFault { from: EndpointId, to: EndpointId },
    #[error("{0} id space exhausted")]
    IdSpaceExhausted(&'static str),
    #[error("Routing error: {0}")]
    Routing(#[from] RoutingError),
}

impl TopologyError {
    /// Maps domain errors onto HTTP-style control-plane status codes.
    pub fn status_code(&self) -> u32 {
        match self {
            Self::EndpointNotFound(_) | Self::RouteNotFound(_) => 404,
            Self::EmptyEndpointName
            | Self::EmptyTargets
            | Self::SelfLoop(_)
            | Self::DuplicateTarget(_)
            | Self::InvalidFaultConfig(_) => 400,
            Self::EndpointAlreadyExists(_) | Self::ConflictingEdgeFault { .. } => 409,
            Self::IdSpaceExhausted(_) => 503,
            Self::Routing(RoutingError::EndpointNotFound(_)) => 404,
            Self::Routing(
                RoutingError::EndpointAlreadyRegistered(_) | RoutingError::EdgeAlreadyExists(_, _),
            ) => 409,
            Self::Routing(_) => 500,
        }
    }
}

/// Deep module owning endpoint/route registries, ID allocation, and `RoutingGraph` sync.
pub struct Topology {
    graph: Arc<RoutingGraph>,
    endpoints: BTreeMap<EndpointId, EndpointInfo>,
    routes: BTreeMap<u32, Route>,
    active_edges: BTreeMap<(EndpointId, EndpointId), Option<FaultConfig>>,
    edge_owners: BTreeMap<(EndpointId, EndpointId), Vec<u32>>,
    next_endpoint_id: u32,
    next_pair_id: u32,
    next_route_id: u32,
}

impl Default for Topology {
    fn default() -> Self {
        Self::new()
    }
}

impl Topology {
    pub fn new() -> Self {
        Self {
            graph: Arc::new(RoutingGraph::new()),
            endpoints: BTreeMap::new(),
            routes: BTreeMap::new(),
            active_edges: BTreeMap::new(),
            edge_owners: BTreeMap::new(),
            next_endpoint_id: 1,
            next_pair_id: 1,
            next_route_id: 100,
        }
    }

    /// Returns the underlying lock-free data-plane [`RoutingGraph`].
    pub fn graph(&self) -> Arc<RoutingGraph> {
        Arc::clone(&self.graph)
    }

    /// Returns whether `id` is currently registered.
    pub fn has_endpoint(&self, id: EndpointId) -> bool {
        self.endpoints.contains_key(&id)
    }

    /// Returns `(active_endpoints, active_routes)`.
    pub fn counts(&self) -> (usize, usize) {
        (self.endpoints.len(), self.routes.len())
    }

    /// Expands all active routes into directed edges.
    pub fn directed_edges(&self) -> Vec<DirectedEdge> {
        self.routes
            .values()
            .flat_map(Route::directed_edges)
            .collect()
    }

    /// Applies one command or query atomically against live endpoints and `RoutingGraph`.
    pub fn apply(&mut self, req: Request) -> Result<Response, TopologyError> {
        match req {
            Request::ListEndpoints => Ok(Response::Endpoints(
                self.endpoints.values().cloned().collect(),
            )),
            Request::ListRoutes => Ok(Response::Routes(self.routes.values().cloned().collect())),
            Request::Snapshot => Ok(Response::Snapshot(TopologySnapshot {
                endpoints: self.endpoints.values().cloned().collect(),
                routes: self.routes.values().cloned().collect(),
            })),
            Request::RegisterEndpoint {
                id,
                name,
                port_type,
                config,
                queue_capacity,
                backpressure,
            } => self.register_endpoint(id, name, port_type, config, queue_capacity, backpressure),
            Request::CreateVirtualPair {
                port_a,
                port_b,
                config,
            } => self.create_virtual_pair(port_a, port_b, config),
            Request::RemoveEndpoint { id } => self.remove_endpoint(id),
            Request::AddRoute {
                source_id,
                target_ids,
                bidirectional,
                fault,
            } => self.add_route(source_id, target_ids, bidirectional, fault),
            Request::RemoveRoute { route_id } => self.remove_route(route_id),
        }
    }

    fn alloc_endpoint_id_after(&self, start: u32) -> Result<EndpointId, TopologyError> {
        let mut candidate = start;
        while candidate <= u16::MAX as u32 {
            let id = candidate as EndpointId;
            if !self.endpoints.contains_key(&id) {
                return Ok(id);
            }
            candidate += 1;
        }
        Err(TopologyError::IdSpaceExhausted("endpoint"))
    }

    fn register_endpoint(
        &mut self,
        id: Option<EndpointId>,
        name: String,
        port_type: String,
        config: SerialConfig,
        queue_capacity: usize,
        backpressure: BackpressurePolicy,
    ) -> Result<Response, TopologyError> {
        if name.trim().is_empty() {
            return Err(TopologyError::EmptyEndpointName);
        }

        let assigned_id = match id {
            Some(explicit) => {
                if self.endpoints.contains_key(&explicit) {
                    return Err(TopologyError::EndpointAlreadyExists(explicit));
                }
                explicit
            }
            None => self.alloc_endpoint_id_after(self.next_endpoint_id)?,
        };

        self.graph
            .register_endpoint(assigned_id, queue_capacity, backpressure)?;

        if id.is_none() {
            self.next_endpoint_id = assigned_id as u32 + 1;
        }

        let info = EndpointInfo {
            id: assigned_id,
            name,
            port_type,
            config,
            is_active: true,
        };
        self.endpoints.insert(assigned_id, info.clone());
        Ok(Response::EndpointRegistered(info))
    }

    fn create_virtual_pair(
        &mut self,
        port_a: String,
        port_b: String,
        config: SerialConfig,
    ) -> Result<Response, TopologyError> {
        if port_a.trim().is_empty() || port_b.trim().is_empty() {
            return Err(TopologyError::EmptyEndpointName);
        }

        // Validate all ID allocations up front so exhaustion never leaves a half-created pair.
        let id_a = self.alloc_endpoint_id_after(self.next_endpoint_id)?;
        let id_b = self.alloc_endpoint_id_after(id_a as u32 + 1)?;
        let pair_id = self.next_pair_id;
        let next_pair_id = pair_id
            .checked_add(1)
            .ok_or(TopologyError::IdSpaceExhausted("virtual pair"))?;
        let route_id = self.next_route_id;
        let next_route_id = route_id
            .checked_add(1)
            .ok_or(TopologyError::IdSpaceExhausted("route"))?;

        self.graph.register_endpoint(
            id_a,
            DEFAULT_QUEUE_CAPACITY,
            BackpressurePolicy::DropNewest,
        )?;
        if let Err(err) = self.graph.register_endpoint(
            id_b,
            DEFAULT_QUEUE_CAPACITY,
            BackpressurePolicy::DropNewest,
        ) {
            let _ = self.graph.unregister_endpoint(id_a);
            return Err(err.into());
        }

        let endpoint_a = EndpointInfo {
            id: id_a,
            name: port_a,
            port_type: "VirtualPairSideA".to_string(),
            config,
            is_active: true,
        };
        let endpoint_b = EndpointInfo {
            id: id_b,
            name: port_b,
            port_type: "VirtualPairSideB".to_string(),
            config,
            is_active: true,
        };
        let route = Route {
            route_id,
            source_id: id_a,
            target_ids: vec![id_b],
            bidirectional: true,
            fault: None,
        };

        let planned = self.plan_route_edges(&route)?;

        self.next_endpoint_id = id_b as u32 + 1;
        self.next_pair_id = next_pair_id;
        self.next_route_id = next_route_id;
        self.endpoints.insert(id_a, endpoint_a.clone());
        self.endpoints.insert(id_b, endpoint_b.clone());
        self.routes.insert(route_id, route.clone());
        self.commit_route_edges(route_id, planned)?;

        Ok(Response::VirtualPairCreated {
            pair_id,
            endpoint_a,
            endpoint_b,
            route,
        })
    }

    fn remove_endpoint(&mut self, id: EndpointId) -> Result<Response, TopologyError> {
        if !self.endpoints.contains_key(&id) {
            return Err(TopologyError::EndpointNotFound(id));
        }

        self.endpoints.remove(&id);
        self.routes.retain(|_, route| route.source_id != id);
        let mut emptied_routes = Vec::new();
        for (&route_id, route) in &mut self.routes {
            route.target_ids.retain(|&target| target != id);
            if route.target_ids.is_empty() {
                emptied_routes.push(route_id);
            }
        }
        for route_id in emptied_routes {
            self.routes.remove(&route_id);
        }

        self.active_edges
            .retain(|&(from, to), _| from != id && to != id);
        self.edge_owners
            .retain(|&(from, to), _| from != id && to != id);
        self.graph.unregister_endpoint(id)?;

        Ok(Response::Removed {
            message: format!("Endpoint {id} removed successfully"),
        })
    }

    fn add_route(
        &mut self,
        source_id: EndpointId,
        target_ids: Vec<EndpointId>,
        bidirectional: bool,
        fault: Option<FaultConfig>,
    ) -> Result<Response, TopologyError> {
        if !self.endpoints.contains_key(&source_id) {
            return Err(TopologyError::EndpointNotFound(source_id));
        }
        if target_ids.is_empty() {
            return Err(TopologyError::EmptyTargets);
        }

        let mut seen = HashSet::with_capacity(target_ids.len());
        for &target_id in &target_ids {
            if !self.endpoints.contains_key(&target_id) {
                return Err(TopologyError::EndpointNotFound(target_id));
            }
            if target_id == source_id {
                return Err(TopologyError::SelfLoop(source_id));
            }
            if !seen.insert(target_id) {
                return Err(TopologyError::DuplicateTarget(target_id));
            }
        }

        if let Some(ref cfg) = fault {
            validate_fault_config(cfg)?;
        }

        let route_id = self.next_route_id;
        let next_route_id = route_id
            .checked_add(1)
            .ok_or(TopologyError::IdSpaceExhausted("route"))?;

        let route = Route {
            route_id,
            source_id,
            target_ids,
            bidirectional,
            fault,
        };

        let planned = self.plan_route_edges(&route)?;

        self.next_route_id = next_route_id;
        self.routes.insert(route_id, route.clone());
        self.commit_route_edges(route_id, planned)?;

        Ok(Response::RouteAdded(route))
    }

    fn remove_route(&mut self, route_id: u32) -> Result<Response, TopologyError> {
        let Some(route) = self.routes.remove(&route_id) else {
            return Err(TopologyError::RouteNotFound(route_id));
        };

        for edge in route.directed_edges() {
            let key = (edge.source_id, edge.target_id);
            enum EdgeAction {
                Keep,
                Remove,
                Replace(Option<FaultConfig>),
            }

            let action = if let Some(owners) = self.edge_owners.get_mut(&key) {
                owners.retain(|&id| id != route_id);
                if owners.is_empty() {
                    EdgeAction::Remove
                } else {
                    let mut folded: Option<FaultConfig> = None;
                    for &owner_id in owners.iter() {
                        if let Some(owner_route) = self.routes.get(&owner_id) {
                            folded = merge_fault(&folded, &owner_route.fault, key.0, key.1)?;
                        }
                    }
                    if self.active_edges.get(&key) == Some(&folded) {
                        EdgeAction::Keep
                    } else {
                        EdgeAction::Replace(folded)
                    }
                }
            } else {
                EdgeAction::Remove
            };

            match action {
                EdgeAction::Keep => {}
                EdgeAction::Remove => {
                    self.edge_owners.remove(&key);
                    self.active_edges.remove(&key);
                    self.graph.remove_edge(key.0, key.1)?;
                }
                EdgeAction::Replace(desired_fault) => {
                    self.graph.remove_edge(key.0, key.1)?;
                    self.graph.add_edge(key.0, key.1, desired_fault.clone())?;
                    self.active_edges.insert(key, desired_fault);
                }
            }
        }

        Ok(Response::Removed {
            message: format!("Route {route_id} removed successfully"),
        })
    }

    /// Computes effective fault configuration for each directed edge in `route`
    /// before mutating any state so conflicts fail atomically.
    fn plan_route_edges(&self, route: &Route) -> Result<Vec<PlannedEdge>, TopologyError> {
        let edges = route.directed_edges();
        let mut planned = Vec::with_capacity(edges.len());
        for edge in edges {
            let key = (edge.source_id, edge.target_id);
            let desired = match self.active_edges.get(&key) {
                None => edge.fault,
                Some(current) => merge_fault(current, &edge.fault, key.0, key.1)?,
            };
            planned.push((key, desired));
        }
        Ok(planned)
    }

    /// Applies planned edges incrementally into `RoutingGraph`, preserving existing
    /// edges whose `FaultConfig` is unchanged.
    fn commit_route_edges(
        &mut self,
        route_id: u32,
        planned: Vec<PlannedEdge>,
    ) -> Result<(), TopologyError> {
        for (key, desired_fault) in planned {
            match self.active_edges.get(&key) {
                Some(current_fault) if current_fault == &desired_fault => {}
                Some(_) => {
                    self.graph.remove_edge(key.0, key.1)?;
                    self.graph.add_edge(key.0, key.1, desired_fault.clone())?;
                    self.active_edges.insert(key, desired_fault);
                }
                None => {
                    self.graph.add_edge(key.0, key.1, desired_fault.clone())?;
                    self.active_edges.insert(key, desired_fault);
                }
            }
            self.edge_owners.entry(key).or_default().push(route_id);
        }
        Ok(())
    }
}

fn validate_fault_config(cfg: &FaultConfig) -> Result<(), TopologyError> {
    fn check_prob(name: &str, value: f64) -> Result<(), TopologyError> {
        if !value.is_finite() || !(0.0..=1.0).contains(&value) {
            return Err(TopologyError::InvalidFaultConfig(format!(
                "{name} must be finite and within 0.0..=1.0, got {value}"
            )));
        }
        Ok(())
    }

    check_prob("bit_error_rate", cfg.bit_error_rate)?;
    if let Some(ge) = cfg.gilbert_elliott {
        check_prob("gilbert_elliott.p_gb", ge.p_gb)?;
        check_prob("gilbert_elliott.p_bg", ge.p_bg)?;
        check_prob("gilbert_elliott.p_g", ge.p_g)?;
        check_prob("gilbert_elliott.p_b", ge.p_b)?;
    }
    Ok(())
}

fn merge_fault(
    current: &Option<FaultConfig>,
    candidate: &Option<FaultConfig>,
    from: EndpointId,
    to: EndpointId,
) -> Result<Option<FaultConfig>, TopologyError> {
    match (current, candidate) {
        (cur, None) => Ok(cur.clone()),
        (None, Some(new_fault)) => Ok(Some(new_fault.clone())),
        (Some(cur), Some(new_fault)) if cur == new_fault => Ok(Some(cur.clone())),
        (Some(_), Some(_)) => Err(TopologyError::ConflictingEdgeFault { from, to }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{FrameFlags, PinState, SerialEvent};
    use bytes::Bytes;
    use std::time::Duration;

    fn make_event(source_id: EndpointId, payload: &[u8]) -> SerialEvent {
        SerialEvent {
            timestamp_ns: 1_000_000,
            source_id,
            flags: FrameFlags::empty(),
            pins: PinState::empty(),
            payload: Bytes::copy_from_slice(payload),
        }
    }

    fn register_mock(topo: &mut Topology, id: Option<EndpointId>, name: &str) -> EndpointInfo {
        match topo
            .apply(Request::RegisterEndpoint {
                id,
                name: name.to_string(),
                port_type: "mock".to_string(),
                config: SerialConfig::default(),
                queue_capacity: 64,
                backpressure: BackpressurePolicy::DropNewest,
            })
            .unwrap()
        {
            Response::EndpointRegistered(info) => info,
            other => panic!("unexpected response: {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_virtual_pair_provisions_endpoints_route_and_live_graph() {
        let mut topo = Topology::new();
        let resp = topo
            .apply(Request::CreateVirtualPair {
                port_a: "COM1".to_string(),
                port_b: "COM2".to_string(),
                config: SerialConfig::default(),
            })
            .unwrap();

        let (ep_a, ep_b, route) = match resp {
            Response::VirtualPairCreated {
                pair_id,
                endpoint_a,
                endpoint_b,
                route,
            } => {
                assert_eq!(pair_id, 1);
                (endpoint_a, endpoint_b, route)
            }
            other => panic!("unexpected response: {other:?}"),
        };

        assert_eq!(ep_a.id, 1);
        assert_eq!(ep_b.id, 2);
        assert_eq!(route.route_id, 100);
        assert!(route.bidirectional);
        assert_eq!(topo.counts(), (2, 1));

        let graph = topo.graph();
        assert_eq!(graph.route(make_event(1, b"ping")).unwrap(), 1);
        assert_eq!(graph.route(make_event(2, b"pong")).unwrap(), 1);
        assert_eq!(&graph.recv(2).await.unwrap().payload[..], b"ping");
        assert_eq!(&graph.recv(1).await.unwrap().payload[..], b"pong");
    }

    #[test]
    fn test_add_route_validates_live_endpoints_and_targets() {
        let mut topo = Topology::new();
        register_mock(&mut topo, Some(1), "ep1");
        register_mock(&mut topo, Some(2), "ep2");

        assert_eq!(
            topo.apply(Request::AddRoute {
                source_id: 99,
                target_ids: vec![2],
                bidirectional: false,
                fault: None,
            }),
            Err(TopologyError::EndpointNotFound(99))
        );
        assert_eq!(
            topo.apply(Request::AddRoute {
                source_id: 1,
                target_ids: vec![2, 88],
                bidirectional: false,
                fault: None,
            }),
            Err(TopologyError::EndpointNotFound(88))
        );
        assert_eq!(
            topo.apply(Request::AddRoute {
                source_id: 1,
                target_ids: vec![],
                bidirectional: false,
                fault: None,
            }),
            Err(TopologyError::EmptyTargets)
        );
        assert_eq!(
            topo.apply(Request::AddRoute {
                source_id: 1,
                target_ids: vec![1],
                bidirectional: false,
                fault: None,
            }),
            Err(TopologyError::SelfLoop(1))
        );
        assert_eq!(
            topo.apply(Request::AddRoute {
                source_id: 1,
                target_ids: vec![2, 2],
                bidirectional: false,
                fault: None,
            }),
            Err(TopologyError::DuplicateTarget(2))
        );
        assert!(matches!(
            topo.apply(Request::AddRoute {
                source_id: 1,
                target_ids: vec![2],
                bidirectional: false,
                fault: Some(FaultConfig {
                    bit_error_rate: 1.5,
                    ..FaultConfig::default()
                }),
            }),
            Err(TopologyError::InvalidFaultConfig(_))
        ));

        // Rejected mutations leave route table and RoutingGraph untouched.
        assert_eq!(topo.counts(), (2, 0));
        assert_eq!(topo.graph().route(make_event(1, b"unrouted")).unwrap(), 0);
    }

    #[tokio::test]
    async fn test_overlapping_routes_never_duplicate_or_break_on_partial_remove() {
        let mut topo = Topology::new();
        topo.apply(Request::CreateVirtualPair {
            port_a: "COM1".to_string(),
            port_b: "COM2".to_string(),
            config: SerialConfig::default(),
        })
        .unwrap();

        let second_route = match topo
            .apply(Request::AddRoute {
                source_id: 1,
                target_ids: vec![2],
                bidirectional: true,
                fault: None,
            })
            .unwrap()
        {
            Response::RouteAdded(route) => route,
            other => panic!("unexpected response: {other:?}"),
        };
        assert_eq!(second_route.route_id, 101);

        // One physical edge in RoutingGraph: frame is delivered once, not twice.
        let graph = topo.graph();
        assert_eq!(graph.route(make_event(1, b"once")).unwrap(), 1);
        assert_eq!(&graph.recv(2).await.unwrap().payload[..], b"once");
        assert!(graph.try_recv(2).unwrap().is_none());

        // Removing the redundant route keeps the default virtual-pair edge live.
        topo.apply(Request::RemoveRoute {
            route_id: second_route.route_id,
        })
        .unwrap();
        assert_eq!(graph.route(make_event(1, b"still_here")).unwrap(), 1);
        assert_eq!(&graph.recv(2).await.unwrap().payload[..], b"still_here");

        // Removing the remaining route tears down the physical edge too.
        topo.apply(Request::RemoveRoute { route_id: 100 }).unwrap();
        assert_eq!(graph.route(make_event(1, b"gone")).unwrap(), 0);
    }

    #[test]
    fn test_conflicting_fault_routes_are_rejected_without_mutation() {
        let mut topo = Topology::new();
        register_mock(&mut topo, Some(1), "ep1");
        register_mock(&mut topo, Some(2), "ep2");

        topo.apply(Request::AddRoute {
            source_id: 1,
            target_ids: vec![2],
            bidirectional: false,
            fault: Some(FaultConfig {
                fixed_delay: Duration::from_millis(5),
                ..FaultConfig::default()
            }),
        })
        .unwrap();

        let err = topo
            .apply(Request::AddRoute {
                source_id: 1,
                target_ids: vec![2],
                bidirectional: false,
                fault: Some(FaultConfig {
                    fixed_delay: Duration::from_millis(25),
                    ..FaultConfig::default()
                }),
            })
            .unwrap_err();
        assert_eq!(err, TopologyError::ConflictingEdgeFault { from: 1, to: 2 });
        assert_eq!(topo.counts(), (2, 1));
    }

    #[tokio::test]
    async fn test_remove_endpoint_prunes_fanout_targets_and_keeps_siblings_live() {
        let mut topo = Topology::new();
        register_mock(&mut topo, Some(1), "src");
        register_mock(&mut topo, Some(2), "dst_a");
        register_mock(&mut topo, Some(3), "dst_b");

        let route = match topo
            .apply(Request::AddRoute {
                source_id: 1,
                target_ids: vec![2, 3],
                bidirectional: true,
                fault: None,
            })
            .unwrap()
        {
            Response::RouteAdded(r) => r,
            other => panic!("unexpected response: {other:?}"),
        };

        topo.apply(Request::RemoveEndpoint { id: 2 }).unwrap();
        assert!(!topo.has_endpoint(2));
        assert_eq!(topo.counts(), (2, 1));

        match topo.apply(Request::ListRoutes).unwrap() {
            Response::Routes(routes) => {
                assert_eq!(routes.len(), 1);
                assert_eq!(routes[0].route_id, route.route_id);
                assert_eq!(routes[0].target_ids, vec![3]);
            }
            other => panic!("unexpected response: {other:?}"),
        }

        // Remaining sibling edge 1 <-> 3 still routes frames cleanly.
        let graph = topo.graph();
        assert_eq!(graph.route(make_event(1, b"to_3")).unwrap(), 1);
        assert_eq!(&graph.recv(3).await.unwrap().payload[..], b"to_3");
        assert!(matches!(
            graph.try_recv(2),
            Err(RoutingError::EndpointNotFound(2))
        ));
    }

    #[test]
    fn test_explicit_u16_max_endpoint_does_not_block_auto_allocation() {
        let mut topo = Topology::new();
        let high = register_mock(&mut topo, Some(u16::MAX), "explicit_high");
        assert_eq!(high.id, u16::MAX);

        let auto = register_mock(&mut topo, None, "auto");
        assert_eq!(auto.id, 1);

        assert_eq!(
            topo.apply(Request::RegisterEndpoint {
                id: Some(1),
                name: "dup".to_string(),
                port_type: "mock".to_string(),
                config: SerialConfig::default(),
                queue_capacity: 16,
                backpressure: BackpressurePolicy::DropNewest,
            }),
            Err(TopologyError::EndpointAlreadyExists(1))
        );
    }

    #[test]
    fn test_id_exhaustion_leaves_topology_unmodified() {
        let mut topo = Topology::new();
        for i in 0..32_767 {
            let resp = topo.apply(Request::CreateVirtualPair {
                port_a: format!("COM{i}"),
                port_b: format!("COM{}", i + 1),
                config: SerialConfig::default(),
            });
            assert!(matches!(resp, Ok(Response::VirtualPairCreated { .. })));
        }

        let err = topo
            .apply(Request::CreateVirtualPair {
                port_a: "COMX".to_string(),
                port_b: "COMY".to_string(),
                config: SerialConfig::default(),
            })
            .unwrap_err();
        assert_eq!(err, TopologyError::IdSpaceExhausted("endpoint"));
        assert_eq!(topo.counts(), (32_767 * 2, 32_767));
    }

    #[tokio::test]
    async fn test_removing_fault_overlay_restores_clean_edge() {
        let mut topo = Topology::new();
        register_mock(&mut topo, Some(1), "src");
        register_mock(&mut topo, Some(2), "dst");

        // Plain route keeps 1 -> 2 flowing.
        topo.apply(Request::AddRoute {
            source_id: 1,
            target_ids: vec![2],
            bidirectional: false,
            fault: None,
        })
        .unwrap();

        // Overlaying a disconnected fault route cuts 1 -> 2.
        let faulty = match topo
            .apply(Request::AddRoute {
                source_id: 1,
                target_ids: vec![2],
                bidirectional: false,
                fault: Some(FaultConfig {
                    disconnected: true,
                    ..FaultConfig::default()
                }),
            })
            .unwrap()
        {
            Response::RouteAdded(r) => r,
            other => panic!("unexpected response: {other:?}"),
        };

        let graph = topo.graph();
        assert_eq!(graph.route(make_event(1, b"dropped")).unwrap(), 0);

        // Removing the faulty overlay downgrades the physical edge back to clean pass-through.
        topo.apply(Request::RemoveRoute {
            route_id: faulty.route_id,
        })
        .unwrap();
        assert_eq!(graph.route(make_event(1, b"restored")).unwrap(), 1);
        assert_eq!(&graph.recv(2).await.unwrap().payload[..], b"restored");
    }
}
