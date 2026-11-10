//! Control-plane seam for the desktop topology editor.
//!
//! `AppTopologyState` (see `topology_commands.rs`) talks to the daemon's
//! `Topology` through this trait, so the WebView never sees transport details.
//! Three adapters exist, mirroring [`crate::capture_store`]:
//!
//! * [`DaemonTopology`] — live `omniportd` over `omniport-proto::RpcClient`
//!   (the default in release builds).
//! * [`FixtureTopology`] — in-process `omniport_core::topology::Topology`,
//!   used only for `tauri dev` / demo sessions so the topology editor remains
//!   interactive without a daemon.
//! * [`DisconnectedTopology`] — explicit host error (initialization failure or
//!   `OMNIPORT_DESKTOP_DISCONNECT`), surfaced to the UI as a banner.

use crate::capture_store::DAEMON_CONNECT_TIMEOUT;
use omniport_core::SerialConfig;
use omniport_core::fault::FaultConfig;
use omniport_core::routing::BackpressurePolicy;
use omniport_core::topology::{
    Request as CoreRequest, Response as CoreResponse, Topology, TopologySnapshot,
};
use omniport_proto::{EndpointInfo, ProtoError, Route, RpcClient};
use std::future::Future;
use std::pin::Pin;
use std::sync::Mutex;
use std::time::Duration;

/// Bounded inbound queue capacity for endpoints created from the desktop UI.
const UI_QUEUE_CAPACITY: usize = 1024;

pub type ControlFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, String>> + Send + 'a>>;

/// Canonical snapshot returned by every topology command.
///
/// The same shape is returned after a mutation so the frontend can replace its
/// graph from one source of truth instead of patching local state.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct TopologyView {
    pub endpoints: Vec<EndpointInfo>,
    pub routes: Vec<Route>,
}

/// Parameters for `create_virtual_pair` (camelCase over the Tauri IPC boundary).
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreatePairRequest {
    pub port_a: String,
    pub port_b: String,
    #[serde(default = "default_baud_rate")]
    pub baud_rate: u32,
    #[serde(default = "default_data_bits")]
    pub data_bits: u8,
    #[serde(default)]
    pub parity: u8,
    #[serde(default = "default_stop_bits")]
    pub stop_bits: u8,
}

impl CreatePairRequest {
    fn serial_config(&self) -> SerialConfig {
        SerialConfig {
            baud_rate: self.baud_rate,
            data_bits: self.data_bits,
            stop_bits: self.stop_bits,
            parity: self.parity,
        }
    }
}

fn default_baud_rate() -> u32 {
    115_200
}

fn default_data_bits() -> u8 {
    8
}

fn default_stop_bits() -> u8 {
    1
}

/// Parameters for `add_route` (camelCase over the Tauri IPC boundary).
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AddRouteRequest {
    pub source_id: u16,
    pub target_ids: Vec<u16>,
    #[serde(default)]
    pub bidirectional: bool,
    #[serde(default)]
    pub fault: Option<FaultConfig>,
}

/// Single seam between the Tauri topology commands and the topology backend.
pub trait TopologyControl: Send + Sync {
    fn snapshot(&self) -> ControlFuture<'_, TopologyView>;

    fn create_pair(&self, request: CreatePairRequest) -> ControlFuture<'_, TopologyView>;

    fn remove_endpoint(&self, endpoint_id: u16) -> ControlFuture<'_, TopologyView>;

    fn add_route(&self, request: AddRouteRequest) -> ControlFuture<'_, TopologyView>;

    fn remove_route(&self, route_id: u32) -> ControlFuture<'_, TopologyView>;
}

fn view_from_core(snapshot: TopologySnapshot) -> TopologyView {
    TopologyView {
        endpoints: snapshot
            .endpoints
            .into_iter()
            .map(EndpointInfo::from)
            .collect(),
        routes: snapshot.routes,
    }
}

// ---------------------------------------------------------------------------
// Daemon adapter
// ---------------------------------------------------------------------------

enum TopologyOp {
    Snapshot,
    CreatePair(CreatePairRequest),
    RemoveEndpoint(u16),
    AddRoute(AddRouteRequest),
    RemoveRoute(u32),
}

/// Live topology adapter communicating with `omniportd` over `omniport-proto`.
pub struct DaemonTopology {
    addr: String,
    client: tokio::sync::Mutex<Option<RpcClient>>,
}

impl DaemonTopology {
    pub fn new(addr: impl Into<String>) -> Self {
        Self {
            addr: addr.into(),
            client: tokio::sync::Mutex::new(None),
        }
    }

    pub fn addr(&self) -> &str {
        &self.addr
    }

    async fn connect(addr: &str) -> Result<RpcClient, String> {
        tokio::time::timeout(DAEMON_CONNECT_TIMEOUT, RpcClient::connect(addr))
            .await
            .map_err(|_| format!("Timed out connecting to omniportd at {addr}"))?
            .map_err(|e| format!("Failed to connect to omniportd at {addr}: {e}"))
    }

    async fn execute(client: &mut RpcClient, op: &TopologyOp) -> Result<TopologyView, ProtoError> {
        match op {
            TopologyOp::Snapshot => {}
            TopologyOp::CreatePair(request) => {
                client
                    .create_virtual_pair(&request.port_a, &request.port_b, request.serial_config())
                    .await?;
            }
            TopologyOp::RemoveEndpoint(endpoint_id) => {
                client.remove_endpoint(*endpoint_id).await?;
            }
            TopologyOp::AddRoute(request) => {
                client
                    .add_route_with_fault(
                        request.source_id,
                        request.target_ids.clone(),
                        request.bidirectional,
                        request.fault.clone(),
                    )
                    .await?;
            }
            TopologyOp::RemoveRoute(route_id) => {
                client.remove_route(*route_id).await?;
            }
        }

        let (endpoints, routes) = client.topology().await?;
        Ok(TopologyView { endpoints, routes })
    }

    /// Runs one operation, reconnecting once when a cached socket has gone stale
    /// (daemon restart) so the UI does not require a manual refresh.
    async fn run(&self, op: TopologyOp) -> Result<TopologyView, String> {
        let mut guard = self.client.lock().await;
        let had_cached_client = guard.is_some();
        if guard.is_none() {
            *guard = Some(Self::connect(&self.addr).await?);
        }

        let result = {
            let client = guard
                .as_mut()
                .ok_or_else(|| "omniportd client unavailable".to_string())?;
            Self::execute(client, &op).await
        };

        match result {
            Ok(view) => Ok(view),
            Err(ProtoError::ConnectionClosed | ProtoError::Io(_)) if had_cached_client => {
                *guard = None;
                *guard = Some(Self::connect(&self.addr).await?);
                let client = guard
                    .as_mut()
                    .ok_or_else(|| "omniportd client unavailable".to_string())?;
                match Self::execute(client, &op).await {
                    Ok(view) => Ok(view),
                    Err(err) => {
                        *guard = None;
                        Err(format!("omniportd topology request failed: {err}"))
                    }
                }
            }
            Err(err) => {
                *guard = None;
                Err(format!("omniportd topology request failed: {err}"))
            }
        }
    }
}

impl TopologyControl for DaemonTopology {
    fn snapshot(&self) -> ControlFuture<'_, TopologyView> {
        Box::pin(self.run(TopologyOp::Snapshot))
    }

    fn create_pair(&self, request: CreatePairRequest) -> ControlFuture<'_, TopologyView> {
        Box::pin(self.run(TopologyOp::CreatePair(request)))
    }

    fn remove_endpoint(&self, endpoint_id: u16) -> ControlFuture<'_, TopologyView> {
        Box::pin(self.run(TopologyOp::RemoveEndpoint(endpoint_id)))
    }

    fn add_route(&self, request: AddRouteRequest) -> ControlFuture<'_, TopologyView> {
        Box::pin(self.run(TopologyOp::AddRoute(request)))
    }

    fn remove_route(&self, route_id: u32) -> ControlFuture<'_, TopologyView> {
        Box::pin(self.run(TopologyOp::RemoveRoute(route_id)))
    }
}

// ---------------------------------------------------------------------------
// Fixture (demo / dev) adapter
// ---------------------------------------------------------------------------

/// In-process topology used by `tauri dev` and explicit demo sessions.
///
/// It runs the real [`omniport_core::topology::Topology`] lifecycle rules, so
/// the desktop demo path exercises the same validation/ID-allocation semantics
/// as the daemon (only the transport and process boundary differ).
pub struct FixtureTopology {
    topology: Mutex<Topology>,
}

impl FixtureTopology {
    /// Creates a demo topology mirroring the static workbench fixtures: host
    /// UART (1), DUT virtual pair (2), Wireshark extcap (3), a bidirectional
    /// 1<->2 route carrying a demo fault, and a 1->3 capture route.
    pub fn seeded() -> Result<Self, String> {
        let mut topology = Topology::new();
        seed_demo_topology(&mut topology)?;
        Ok(Self {
            topology: Mutex::new(topology),
        })
    }

    /// Wraps an existing topology (for unit/integration tests).
    pub fn from_topology(topology: Topology) -> Self {
        Self {
            topology: Mutex::new(topology),
        }
    }

    fn snapshot_locked(&self) -> Result<TopologyView, String> {
        let mut guard = self.topology.lock().map_err(|e| e.to_string())?;
        match guard
            .apply(CoreRequest::Snapshot)
            .map_err(|e| e.to_string())?
        {
            CoreResponse::Snapshot(snapshot) => Ok(view_from_core(snapshot)),
            other => Err(format!("unexpected topology response: {other:?}")),
        }
    }

    fn apply_then_snapshot(&self, request: CoreRequest) -> Result<TopologyView, String> {
        {
            let mut guard = self.topology.lock().map_err(|e| e.to_string())?;
            guard.apply(request).map_err(|e| e.to_string())?;
        }
        self.snapshot_locked()
    }
}

impl TopologyControl for FixtureTopology {
    fn snapshot(&self) -> ControlFuture<'_, TopologyView> {
        Box::pin(async move { self.snapshot_locked() })
    }

    fn create_pair(&self, request: CreatePairRequest) -> ControlFuture<'_, TopologyView> {
        Box::pin(async move {
            let config = request.serial_config();
            self.apply_then_snapshot(CoreRequest::CreateVirtualPair {
                port_a: request.port_a,
                port_b: request.port_b,
                config,
            })
        })
    }

    fn remove_endpoint(&self, endpoint_id: u16) -> ControlFuture<'_, TopologyView> {
        Box::pin(async move {
            self.apply_then_snapshot(CoreRequest::RemoveEndpoint { id: endpoint_id })
        })
    }

    fn add_route(&self, request: AddRouteRequest) -> ControlFuture<'_, TopologyView> {
        Box::pin(async move {
            self.apply_then_snapshot(CoreRequest::AddRoute {
                source_id: request.source_id,
                target_ids: request.target_ids,
                bidirectional: request.bidirectional,
                fault: request.fault,
            })
        })
    }

    fn remove_route(&self, route_id: u32) -> ControlFuture<'_, TopologyView> {
        Box::pin(async move { self.apply_then_snapshot(CoreRequest::RemoveRoute { route_id }) })
    }
}

fn register_fixture_endpoint(
    topology: &mut Topology,
    id: u16,
    name: &str,
    port_type: &str,
    baud_rate: u32,
) -> Result<(), String> {
    topology
        .apply(CoreRequest::RegisterEndpoint {
            id: Some(id),
            name: name.to_string(),
            port_type: port_type.to_string(),
            config: SerialConfig {
                baud_rate,
                data_bits: 8,
                stop_bits: 1,
                parity: 0,
            },
            queue_capacity: UI_QUEUE_CAPACITY,
            backpressure: BackpressurePolicy::DropNewest,
        })
        .map_err(|e| e.to_string())?;
    Ok(())
}

fn seed_demo_topology(topology: &mut Topology) -> Result<(), String> {
    register_fixture_endpoint(topology, 1, "COM1", "uart", 115_200)?;
    register_fixture_endpoint(topology, 2, "COM3", "virtual_pair", 115_200)?;
    register_fixture_endpoint(topology, 3, r"\\.\pipe\omniport_extcap", "pty", 921_600)?;

    let demo_fault = FaultConfig {
        fixed_delay: Duration::from_millis(5),
        jitter_mean: Duration::from_millis(2),
        jitter_std_dev: Duration::from_micros(1_500),
        rate_limit_bps: None,
        disconnected: false,
        bit_error_rate: 1e-4,
        gilbert_elliott: None,
        corrupt_nth_crc: None,
    };

    topology
        .apply(CoreRequest::AddRoute {
            source_id: 1,
            target_ids: vec![2],
            bidirectional: true,
            fault: Some(demo_fault),
        })
        .map_err(|e| e.to_string())?;
    topology
        .apply(CoreRequest::AddRoute {
            source_id: 1,
            target_ids: vec![3],
            bidirectional: false,
            fault: None,
        })
        .map_err(|e| e.to_string())?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Disconnected adapter
// ---------------------------------------------------------------------------

/// Explicit error adapter surfaced when the topology backend is unavailable.
pub struct DisconnectedTopology {
    reason: String,
}

impl DisconnectedTopology {
    pub fn new(reason: impl Into<String>) -> Self {
        Self {
            reason: reason.into(),
        }
    }

    fn fail<T>(&self) -> Result<T, String> {
        Err(self.reason.clone())
    }
}

impl TopologyControl for DisconnectedTopology {
    fn snapshot(&self) -> ControlFuture<'_, TopologyView> {
        Box::pin(async move { self.fail() })
    }

    fn create_pair(&self, _request: CreatePairRequest) -> ControlFuture<'_, TopologyView> {
        Box::pin(async move { self.fail() })
    }

    fn remove_endpoint(&self, _endpoint_id: u16) -> ControlFuture<'_, TopologyView> {
        Box::pin(async move { self.fail() })
    }

    fn add_route(&self, _request: AddRouteRequest) -> ControlFuture<'_, TopologyView> {
        Box::pin(async move { self.fail() })
    }

    fn remove_route(&self, _route_id: u32) -> ControlFuture<'_, TopologyView> {
        Box::pin(async move { self.fail() })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pair_request(port_a: &str, port_b: &str) -> CreatePairRequest {
        CreatePairRequest {
            port_a: port_a.to_string(),
            port_b: port_b.to_string(),
            baud_rate: 115_200,
            data_bits: 8,
            parity: 0,
            stop_bits: 1,
        }
    }

    #[tokio::test]
    async fn test_fixture_topology_seeded_snapshot_matches_demo_workbench() {
        let topology = FixtureTopology::seeded().expect("seeded fixture topology");
        let view = topology.snapshot().await.unwrap();

        assert_eq!(view.endpoints.len(), 3);
        assert_eq!(view.endpoints[0].id, 1);
        assert_eq!(view.endpoints[0].name, "COM1");
        assert_eq!(view.endpoints[0].port_type, "uart");
        assert_eq!(view.endpoints[0].baud_rate, 115_200);
        assert!(view.endpoints[0].is_active);

        assert_eq!(view.routes.len(), 2);
        let routed = view
            .routes
            .iter()
            .find(|route| route.target_ids == vec![2])
            .expect("1<->2 route present");
        assert!(routed.bidirectional);
        assert!(routed.fault.is_some(), "demo route carries a fault");

        let capture_route = view
            .routes
            .iter()
            .find(|route| route.target_ids == vec![3])
            .expect("1->3 route present");
        assert!(!capture_route.bidirectional);
        assert!(capture_route.fault.is_none());
    }

    #[tokio::test]
    async fn test_fixture_topology_mutations_roundtrip() {
        let topology = FixtureTopology::seeded().unwrap();

        let after_pair = topology
            .create_pair(pair_request("COM10", "COM11"))
            .await
            .unwrap();
        assert_eq!(after_pair.endpoints.len(), 5);
        assert_eq!(after_pair.routes.len(), 3);
        let created = after_pair
            .routes
            .iter()
            .find(|route| route.target_ids.len() == 1 && route.source_id >= 4)
            .expect("virtual pair route created");
        assert!(created.bidirectional);

        let endpoint_a = after_pair
            .endpoints
            .iter()
            .find(|ep| ep.name == "COM10")
            .unwrap()
            .id;
        let endpoint_b = after_pair
            .endpoints
            .iter()
            .find(|ep| ep.name == "COM11")
            .unwrap()
            .id;

        let after_route = topology
            .add_route(AddRouteRequest {
                source_id: 1,
                target_ids: vec![endpoint_a],
                bidirectional: false,
                fault: None,
            })
            .await
            .unwrap();
        assert_eq!(after_route.routes.len(), 4);

        let custom_route = after_route
            .routes
            .iter()
            .find(|route| route.source_id == 1 && route.target_ids == vec![endpoint_a])
            .expect("custom route present")
            .route_id;
        let after_remove_route = topology.remove_route(custom_route).await.unwrap();
        assert_eq!(after_remove_route.routes.len(), 3);

        let after_remove_endpoint = topology.remove_endpoint(endpoint_b).await.unwrap();
        assert!(
            after_remove_endpoint
                .endpoints
                .iter()
                .all(|ep| ep.id != endpoint_b)
        );
        assert!(
            after_remove_endpoint
                .routes
                .iter()
                .all(|route| !route.target_ids.contains(&endpoint_b)
                    && route.source_id != endpoint_b)
        );
    }

    #[tokio::test]
    async fn test_fixture_topology_rejects_invalid_mutations() {
        let topology = FixtureTopology::seeded().unwrap();

        let empty_name = topology.create_pair(pair_request("  ", "COM9")).await;
        assert!(empty_name.is_err());

        let unknown_source = topology
            .add_route(AddRouteRequest {
                source_id: 42,
                target_ids: vec![1],
                bidirectional: false,
                fault: None,
            })
            .await;
        assert!(unknown_source.unwrap_err().contains("not found"));

        let self_loop = topology
            .add_route(AddRouteRequest {
                source_id: 1,
                target_ids: vec![1],
                bidirectional: false,
                fault: None,
            })
            .await;
        assert!(
            self_loop
                .unwrap_err()
                .to_lowercase()
                .contains("source endpoint"),
            "self-loop rejection should name the source endpoint"
        );

        let missing_route = topology.remove_route(999_999).await;
        assert!(missing_route.is_err());
    }

    #[tokio::test]
    async fn test_disconnected_topology_surfaces_reason() {
        let topology = DisconnectedTopology::new("host topology unavailable");
        assert_eq!(
            topology.snapshot().await.unwrap_err(),
            "host topology unavailable"
        );
        assert_eq!(
            topology.remove_route(1).await.unwrap_err(),
            "host topology unavailable"
        );
    }
}
