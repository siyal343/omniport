//! Integration tests for the Step 7.1 topology command surface.
//!
//! These exercise the same `AppTopologyState` the Tauri commands delegate to,
//! against a real in-process `omniportd` (daemon adapter) and the in-process
//! fixture adapter used by `tauri dev` / demo sessions.

use omniport_desktop_lib::{
    AddRouteRequest, AppTopologyState, CreatePairRequest, DaemonTopology, FixtureTopology,
};
use omniport_proto::RpcClient;
use omniportd::{DaemonEngine, run_server};
use std::sync::Arc;
use std::time::Duration;
use tokio::net::{TcpListener, TcpStream};

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

async fn allocate_ephemeral_addr() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    drop(listener);
    addr
}

async fn wait_until_listening(addr: &str) {
    for _ in 0..40 {
        if TcpStream::connect(addr).await.is_ok() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("omniportd test server failed to start listening on {addr}");
}

async fn start_daemon(addr: &str) -> tokio::task::JoinHandle<()> {
    let engine = Arc::new(DaemonEngine::new());
    let server_addr = addr.to_string();
    let handle = tokio::spawn(async move {
        run_server(&server_addr, engine).await.unwrap();
    });
    wait_until_listening(addr).await;
    handle
}

async fn shutdown_daemon(addr: &str, handle: tokio::task::JoinHandle<()>) {
    let mut control = RpcClient::connect(addr).await.unwrap();
    control.shutdown().await.unwrap();
    drop(control);
    handle.await.unwrap();
}

#[tokio::test]
async fn test_daemon_topology_command_surface_roundtrip() {
    let addr = allocate_ephemeral_addr().await;
    let server = start_daemon(&addr).await;
    let state = AppTopologyState::new(Arc::new(DaemonTopology::new(&addr)));

    // A fresh daemon exposes an empty topology.
    let empty = state.snapshot().await.unwrap();
    assert!(empty.endpoints.is_empty());
    assert!(empty.routes.is_empty());

    // Create a virtual pair: two endpoints plus their bidirectional route.
    let first_pair = state
        .create_pair(pair_request("COM10", "COM11"))
        .await
        .unwrap();
    assert_eq!(first_pair.endpoints.len(), 2);
    assert_eq!(first_pair.routes.len(), 1);
    let first_route = &first_pair.routes[0];
    assert!(first_route.bidirectional);
    assert_eq!(first_route.target_ids.len(), 1);
    let first_ids: Vec<u16> = first_pair.endpoints.iter().map(|ep| ep.id).collect();
    assert_eq!(first_pair.endpoints[0].name, "COM10");
    assert_eq!(first_pair.endpoints[1].name, "COM11");
    assert!(
        first_pair.endpoints[0]
            .port_type
            .contains("VirtualPairSide")
    );

    // A second pair gives the graph two live endpoints to route between.
    let second_pair = state
        .create_pair(pair_request("COM12", "COM13"))
        .await
        .unwrap();
    assert_eq!(second_pair.endpoints.len(), 4);
    assert_eq!(second_pair.routes.len(), 2);
    let second_ids: Vec<u16> = second_pair
        .endpoints
        .iter()
        .filter(|ep| ep.name == "COM12" || ep.name == "COM13")
        .map(|ep| ep.id)
        .collect();
    assert_eq!(second_ids.len(), 2);

    // Add a directed route between the two pairs.
    let after_route = state
        .add_route(AddRouteRequest {
            source_id: second_ids[0],
            target_ids: vec![first_ids[0]],
            bidirectional: false,
            fault: None,
        })
        .await
        .unwrap();
    assert_eq!(after_route.routes.len(), 3);
    let custom_route = after_route
        .routes
        .iter()
        .find(|route| route.source_id == second_ids[0] && route.target_ids == vec![first_ids[0]])
        .expect("custom route present")
        .route_id;

    // Remove just that route.
    let after_remove_route = state.remove_route(custom_route).await.unwrap();
    assert_eq!(after_remove_route.routes.len(), 2);

    // Removing an endpoint prunes its routes atomically.
    let after_remove_endpoint = state.remove_endpoint(second_ids[1]).await.unwrap();
    assert_eq!(after_remove_endpoint.endpoints.len(), 3);
    assert!(after_remove_endpoint.routes.iter().all(
        |route| route.source_id != second_ids[1] && !route.target_ids.contains(&second_ids[1])
    ));
    // The second pair's only route lost its remaining target and was pruned.
    assert_eq!(after_remove_endpoint.routes.len(), 1);

    // Invalid operations fail with actionable messages instead of panicking.
    assert!(
        state
            .create_pair(pair_request("   ", "COM14"))
            .await
            .is_err()
    );
    assert!(
        state
            .add_route(AddRouteRequest {
                source_id: 42_424,
                target_ids: vec![first_ids[0]],
                bidirectional: false,
                fault: None,
            })
            .await
            .unwrap_err()
            .contains("not found")
    );
    assert!(state.remove_route(999_999).await.is_err());

    shutdown_daemon(&addr, server).await;

    // After shutdown the cached connection must surface an error, not stale data.
    assert!(state.snapshot().await.is_err());
}

#[tokio::test]
async fn test_daemon_topology_state_reconnects_after_restart() {
    let addr = allocate_ephemeral_addr().await;
    let first_server = start_daemon(&addr).await;
    let state = AppTopologyState::new(Arc::new(DaemonTopology::new(&addr)));

    let created = state
        .create_pair(pair_request("COM20", "COM21"))
        .await
        .unwrap();
    assert_eq!(created.endpoints.len(), 2);

    shutdown_daemon(&addr, first_server).await;

    // A fresh daemon instance starts with an empty topology. The cached socket is
    // stale, so the adapter must reconnect once and report the new snapshot.
    let second_server = start_daemon(&addr).await;
    let after_restart = state.snapshot().await.unwrap();
    assert!(after_restart.endpoints.is_empty());
    assert!(after_restart.routes.is_empty());

    shutdown_daemon(&addr, second_server).await;
}

#[tokio::test]
async fn test_fixture_topology_state_supports_demo_editing() {
    let state = AppTopologyState::new(Arc::new(FixtureTopology::seeded().unwrap()));

    let seeded = state.snapshot().await.unwrap();
    assert_eq!(seeded.endpoints.len(), 3);
    assert_eq!(seeded.routes.len(), 2);

    let created = state
        .create_pair(pair_request("COM30", "COM31"))
        .await
        .unwrap();
    assert_eq!(created.endpoints.len(), 5);
    let created_ids: Vec<u16> = created
        .endpoints
        .iter()
        .filter(|ep| ep.name == "COM30" || ep.name == "COM31")
        .map(|ep| ep.id)
        .collect();
    assert_eq!(created_ids.len(), 2);

    let routed = state
        .add_route(AddRouteRequest {
            source_id: created_ids[0],
            target_ids: vec![1],
            bidirectional: false,
            fault: None,
        })
        .await
        .unwrap();
    assert_eq!(routed.routes.len(), 4);

    let removed = state.remove_endpoint(created_ids[1]).await.unwrap();
    assert_eq!(removed.endpoints.len(), 4);
}
