use bytes::Bytes;
use omniport_capture::ViewportBatch;
use omniport_core::{FrameFlags, PinState, SerialEvent};
use omniport_proto::{RpcClient, SerialConfigDto};
use omniportd::{DaemonEngine, run_server};
use std::sync::Arc;
use tokio::net::TcpListener;

#[tokio::test]
async fn test_daemon_client_rpc_roundtrip() {
    // Bind to ephemeral port
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener); // Free for run_server

    let engine = Arc::new(DaemonEngine::new());
    let engine_clone = Arc::clone(&engine);
    let addr_str = addr.to_string();

    let server_handle = tokio::spawn(async move {
        run_server(&addr_str, engine_clone).await.unwrap();
    });

    // Give server time to bind
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let mut client = RpcClient::connect(&addr.to_string()).await.unwrap();

    // 1. Health check
    let (status, _uptime, active_eps, active_routes) = client.health_check().await.unwrap();
    assert_eq!(status, "RUNNING");
    assert_eq!(active_eps, 0);
    assert_eq!(active_routes, 0);

    // 2. Create virtual pair
    let cfg = SerialConfigDto {
        baud_rate: 115_200,
        data_bits: 8,
        stop_bits: 1,
        parity: 0,
    };
    let pair_id = client
        .create_virtual_pair("COM40", "COM41", cfg)
        .await
        .unwrap();
    assert_eq!(pair_id, 1);

    // 3. List endpoints
    let eps = client.list_endpoints().await.unwrap();
    assert_eq!(eps.len(), 2);
    assert!(eps.iter().any(|e| e.name == "COM40"));
    assert!(eps.iter().any(|e| e.name == "COM41"));

    // 4. Add route
    let route_id = client.add_route(1, vec![2], true).await.unwrap();
    assert_eq!(route_id, 101); // 100 was default 1:1 route for the pair

    // 5. Remove route
    client.remove_route(route_id).await.unwrap();

    // 6. Remove endpoint
    client.remove_endpoint(1).await.unwrap();
    let eps_after = client.list_endpoints().await.unwrap();
    assert_eq!(eps_after.len(), 1);

    // 7. Capture viewport & status roundtrip
    engine
        .record_event(&SerialEvent {
            timestamp_ns: 42_000_000,
            source_id: 2,
            flags: FrameFlags::DIR_TX,
            pins: PinState::RTS | PinState::DTR,
            payload: Bytes::from_static(b"DAEMON_FRAME"),
        })
        .await;
    let (total_packets, dropped_frames) = client.capture_status().await.unwrap();
    assert_eq!(total_packets, 1);
    assert_eq!(dropped_frames, 0);

    let wire = client.read_viewport(0, 10).await.unwrap();
    let batch = ViewportBatch::decode(&wire).unwrap();
    assert_eq!(batch.total_packets(), 1);
    assert_eq!(batch.returned_rows(), 1);
    assert_eq!(batch.rows()[0].endpoint_id, 2);
    assert_eq!(batch.rows()[0].payload, b"DAEMON_FRAME");

    // 8. Shutdown
    client.shutdown().await.unwrap();
    server_handle.await.unwrap();
}
