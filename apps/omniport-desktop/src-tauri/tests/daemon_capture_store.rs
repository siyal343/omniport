use bytes::Bytes;
use omniport_capture::ViewportBatch;
use omniport_core::{FrameFlags, PinState, SerialEvent};
use omniport_desktop_lib::{AppCaptureState, DaemonCaptureStore};
use omniport_proto::RpcClient;
use omniportd::{DaemonEngine, run_server};
use std::sync::Arc;
use std::time::Duration;
use tokio::net::{TcpListener, TcpStream};

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

#[tokio::test]
async fn test_daemon_capture_store_roundtrip_and_shutdown() {
    let addr = allocate_ephemeral_addr().await;
    let engine = Arc::new(DaemonEngine::new());
    let server_engine = Arc::clone(&engine);
    let server_addr = addr.clone();

    let server_handle = tokio::spawn(async move {
        run_server(&server_addr, server_engine).await.unwrap();
    });
    wait_until_listening(&addr).await;

    let state = AppCaptureState::new(Arc::new(DaemonCaptureStore::new(&addr)));

    // Empty live daemon store reports demo_mode=false and 0 packets (never an error).
    let initial_status = state.status().await.unwrap();
    assert!(!initial_status.demo_mode);
    assert_eq!(initial_status.total_packets, 0);
    assert_eq!(initial_status.dropped_frames, 0);

    let empty_wire = state.read_viewport(0, 20).await.unwrap();
    let empty_batch = ViewportBatch::decode(&empty_wire).unwrap();
    assert_eq!(empty_batch.total_packets(), 0);
    assert_eq!(empty_batch.returned_rows(), 0);

    // Record real serial events inside omniportd.
    let expected = [
        (
            1_000_000_000u64,
            1u16,
            FrameFlags::DIR_TX,
            PinState::RTS | PinState::DTR,
            b"PING_0".as_slice(),
        ),
        (
            1_005_000_000,
            2,
            FrameFlags::empty(),
            PinState::CTS | PinState::DCD,
            b"PONG_1",
        ),
        (
            1_010_000_000,
            1,
            FrameFlags::DIR_TX | FrameFlags::ERR_CRC,
            PinState::RTS | PinState::DSR,
            b"CORRUPT_2",
        ),
        (
            1_015_000_000,
            3,
            FrameFlags::FAULT_INJECT,
            PinState::RI,
            b"DELAYED_3",
        ),
        (
            1_020_000_000,
            2,
            FrameFlags::PIN_CHANGE,
            PinState::BREAK,
            b"",
        ),
    ];
    for (timestamp_ns, source_id, flags, pins, payload) in expected {
        engine
            .record_event(&SerialEvent {
                timestamp_ns,
                source_id,
                flags,
                pins,
                payload: Bytes::copy_from_slice(payload),
            })
            .await;
    }

    let status = state.status().await.unwrap();
    assert!(!status.demo_mode);
    assert_eq!(status.total_packets, 5);
    assert_eq!(status.dropped_frames, 0);

    // Read a middle viewport slice through the Tauri state seam.
    let wire = state.read_viewport(1, 3).await.unwrap();
    let batch = ViewportBatch::decode(&wire).unwrap();
    assert_eq!(batch.total_packets(), 5);
    assert_eq!(batch.returned_rows(), 3);
    for (row, &(ts, ep, flags, pins, payload)) in batch.rows().iter().zip(&expected[1..4]) {
        assert_eq!(row.timestamp_ns, ts);
        assert_eq!(row.endpoint_id, ep);
        assert_eq!(row.flags, flags);
        assert_eq!(row.pins, pins);
        assert_eq!(row.payload, payload);
    }

    // Out-of-range start_row and oversized row_count remain safe.
    let oob_wire = state.read_viewport(500, 10).await.unwrap();
    let oob_batch = ViewportBatch::decode(&oob_wire).unwrap();
    assert_eq!(oob_batch.total_packets(), 5);
    assert_eq!(oob_batch.returned_rows(), 0);

    let clamped_wire = state.read_viewport(0, usize::MAX).await.unwrap();
    let clamped = ViewportBatch::decode(&clamped_wire).unwrap();
    assert_eq!(clamped.total_packets(), 5);
    assert_eq!(clamped.returned_rows(), 5);

    // Shut down omniportd and verify the GUI seam surfaces errors instead of demo fallback.
    let mut ctl = RpcClient::connect(&addr).await.unwrap();
    ctl.shutdown().await.unwrap();
    drop(ctl);
    server_handle.await.unwrap();

    assert!(state.read_viewport(0, 10).await.is_err());
    assert!(state.status().await.is_err());
}

#[tokio::test]
async fn test_daemon_capture_store_recovers_after_daemon_starts() {
    let addr = allocate_ephemeral_addr().await;
    let state = AppCaptureState::new(Arc::new(DaemonCaptureStore::new(&addr)));

    // Daemon not running yet -> explicit error.
    assert!(state.status().await.is_err());

    // Start daemon on that address and record one frame.
    let engine = Arc::new(DaemonEngine::new());
    engine
        .record_event(&SerialEvent {
            timestamp_ns: 77_000,
            source_id: 4,
            flags: FrameFlags::DIR_TX,
            pins: PinState::DTR,
            payload: Bytes::from_static(b"RECOVERED"),
        })
        .await;

    let server_engine = Arc::clone(&engine);
    let server_addr = addr.clone();
    let server_handle = tokio::spawn(async move {
        run_server(&server_addr, server_engine).await.unwrap();
    });
    wait_until_listening(&addr).await;

    // Retry succeeds against the newly started daemon.
    let status = state.status().await.unwrap();
    assert!(!status.demo_mode);
    assert_eq!(status.total_packets, 1);

    let wire = state.read_viewport(0, 5).await.unwrap();
    let batch = ViewportBatch::decode(&wire).unwrap();
    assert_eq!(batch.returned_rows(), 1);
    assert_eq!(batch.rows()[0].payload, b"RECOVERED");

    let mut ctl = RpcClient::connect(&addr).await.unwrap();
    ctl.shutdown().await.unwrap();
    drop(ctl);
    server_handle.await.unwrap();
}

#[tokio::test]
async fn test_daemon_capture_store_reconnects_after_daemon_restart() {
    let addr = allocate_ephemeral_addr().await;
    let engine1 = Arc::new(DaemonEngine::new());
    engine1
        .record_event(&SerialEvent {
            timestamp_ns: 10_000,
            source_id: 1,
            flags: FrameFlags::DIR_TX,
            pins: PinState::RTS,
            payload: Bytes::from_static(b"BEFORE_RESTART"),
        })
        .await;

    let srv1_engine = Arc::clone(&engine1);
    let srv1_addr = addr.clone();
    let srv1 = tokio::spawn(async move {
        run_server(&srv1_addr, srv1_engine).await.unwrap();
    });
    wait_until_listening(&addr).await;

    let state = AppCaptureState::new(Arc::new(DaemonCaptureStore::new(&addr)));
    let wire1 = state.read_viewport(0, 10).await.unwrap();
    assert_eq!(
        ViewportBatch::decode(&wire1).unwrap().rows()[0].payload,
        b"BEFORE_RESTART"
    );

    // Shut down the first daemon instance while the store still caches its TCP connection.
    let mut ctl1 = RpcClient::connect(&addr).await.unwrap();
    ctl1.shutdown().await.unwrap();
    drop(ctl1);
    srv1.await.unwrap();

    // Start a fresh daemon on the same address with new capture data.
    let engine2 = Arc::new(DaemonEngine::new());
    engine2
        .record_event(&SerialEvent {
            timestamp_ns: 20_000,
            source_id: 2,
            flags: FrameFlags::empty(),
            pins: PinState::CTS,
            payload: Bytes::from_static(b"AFTER_RESTART"),
        })
        .await;

    let srv2_engine = Arc::clone(&engine2);
    let srv2_addr = addr.clone();
    let srv2 = tokio::spawn(async move {
        run_server(&srv2_addr, srv2_engine).await.unwrap();
    });
    wait_until_listening(&addr).await;

    // The cached-connection stale-socket retry reconnects transparently on the next call.
    let status = state.status().await.unwrap();
    assert!(!status.demo_mode);
    assert_eq!(status.total_packets, 1);

    let wire2 = state.read_viewport(0, 10).await.unwrap();
    assert_eq!(
        ViewportBatch::decode(&wire2).unwrap().rows()[0].payload,
        b"AFTER_RESTART"
    );

    let mut ctl2 = RpcClient::connect(&addr).await.unwrap();
    ctl2.shutdown().await.unwrap();
    drop(ctl2);
    srv2.await.unwrap();
}
