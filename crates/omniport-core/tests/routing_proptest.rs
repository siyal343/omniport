use bytes::Bytes;
use omniport_core::fault::{FaultConfig, FaultInjector};
use omniport_core::routing::{BackpressurePolicy, BoundedEventQueue, RoutingGraph};
use omniport_core::{FrameFlags, PinState, SerialEvent};
use proptest::prelude::*;
use std::sync::Arc;
use std::time::Duration;

fn make_event(source: u16, payload: &[u8]) -> SerialEvent {
    SerialEvent {
        timestamp_ns: 1_000_000,
        source_id: source,
        flags: FrameFlags::empty(),
        pins: PinState::empty(),
        payload: Bytes::copy_from_slice(payload),
    }
}

proptest! {
    #![proptest_config(ProptestConfig { failure_persistence: None, cases: 50, ..ProptestConfig::default() })]

    /// FR-202 / Invariant 3: 1:N fanout routes identical payloads and preserves zero-copy buffer pointers
    #[test]
    fn prop_1_to_n_fanout_zero_copy(
        payload in prop::collection::vec(any::<u8>(), 1..512),
        dest_count in 1u16..8
    ) {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let graph = RoutingGraph::new();
            graph.register_endpoint(1, 100, BackpressurePolicy::DropNewest).unwrap();

            for d in 2..=(dest_count + 1) {
                graph.register_endpoint(d, 100, BackpressurePolicy::DropNewest).unwrap();
                graph.add_edge(1, d, None).unwrap();
            }

            let event = make_event(1, &payload);
            let orig_ptr = event.payload.as_ptr();

            let dispatched = graph.route(event).unwrap();
            prop_assert_eq!(dispatched, dest_count as usize);

            for d in 2..=(dest_count + 1) {
                let recv = graph.recv(d).await.unwrap();
                prop_assert_eq!(&recv.payload[..], &payload[..]);
                prop_assert_eq!(recv.payload.as_ptr(), orig_ptr, "Buffer pointer must match (zero-copy)");
            }
            Ok(())
        }).unwrap();
    }

    /// FR-302: Targeted N-th frame CRC corruption property
    #[test]
    fn prop_targeted_nth_crc_corruption(
        total_frames in 10u64..40,
        nth in 2u64..6
    ) {
        let mut injector = FaultInjector::new(FaultConfig {
            corrupt_nth_crc: Some(nth),
            ..Default::default()
        });

        for i in 1..=total_frames {
            let out = injector.inject(make_event(1, b"PACKET_DATA_BYTES"), 1_000_000).unwrap();
            if i % nth == 0 {
                prop_assert!(out.event.flags.contains(FrameFlags::ERR_CRC | FrameFlags::FAULT_INJECT));
                prop_assert_ne!(&out.event.payload[..], b"PACKET_DATA_BYTES");
            } else {
                prop_assert_eq!(out.event.flags, FrameFlags::empty());
                prop_assert_eq!(&out.event.payload[..], b"PACKET_DATA_BYTES");
            }
        }
    }

    /// FR-203: Bounded queue DropNewest and DropOldest capacity invariants
    #[test]
    fn prop_queue_backpressure_capacity(
        capacity in 2usize..10,
        extra_pushes in 1usize..15
    ) {
        let q_newest = BoundedEventQueue::new(capacity, BackpressurePolicy::DropNewest);
        let q_oldest = BoundedEventQueue::new(capacity, BackpressurePolicy::DropOldest);

        let total = capacity + extra_pushes;
        for i in 0..total {
            let s = i.to_string();
            let _ = q_newest.push(make_event(1, s.as_bytes()));
            let _ = q_oldest.push(make_event(1, s.as_bytes()));
        }

        // Neither queue may exceed configured capacity
        prop_assert_eq!(q_newest.len(), capacity);
        prop_assert_eq!(q_oldest.len(), capacity);

        // DropNewest must contain first `capacity` elements (0..capacity)
        for i in 0..capacity {
            let item = q_newest.try_recv(1).unwrap().unwrap();
            let exp = i.to_string();
            prop_assert_eq!(&item.payload[..], exp.as_bytes());
        }

        // DropOldest must contain last `capacity` elements ((total - capacity)..total)
        let oldest_start = total - capacity;
        for i in oldest_start..total {
            let item = q_oldest.try_recv(1).unwrap().unwrap();
            let exp = i.to_string();
            prop_assert_eq!(&item.payload[..], exp.as_bytes());
        }
    }
}

/// FR-203 / VV_RTM: Atomic frame merging under high concurrent contention (zero byte interleaving)
#[tokio::test]
async fn test_n_to_1_atomic_frame_merger_concurrency() {
    let graph = Arc::new(RoutingGraph::new());
    let dest_id = 99;
    graph
        .register_endpoint(dest_id, 2000, BackpressurePolicy::DropNewest)
        .unwrap();

    let sender_count: usize = 8;
    let frames_per_sender: usize = 50;

    for s in 1..=sender_count {
        graph
            .register_endpoint(s as u16, 100, BackpressurePolicy::DropNewest)
            .unwrap();
        graph.add_edge(s as u16, dest_id, None).unwrap();
    }

    let mut handles = Vec::new();
    for s in 1..=sender_count {
        let g = Arc::clone(&graph);
        handles.push(tokio::spawn(async move {
            for f in 0..frames_per_sender {
                let payload = format!("SRC_{:02}_FRAME_{:04}", s, f);
                g.route(make_event(s as u16, payload.as_bytes())).unwrap();
                tokio::task::yield_now().await;
            }
        }));
    }

    for h in handles {
        h.await.unwrap();
    }

    let total_expected = sender_count * frames_per_sender;
    let mut received = Vec::with_capacity(total_expected);

    for _ in 0..total_expected {
        let evt = graph.recv(dest_id).await.unwrap();
        let s = String::from_utf8(evt.payload.to_vec()).unwrap();
        // Strict assertion: exactly formatted frame, zero interleaved bytes
        assert!(
            s.starts_with("SRC_") && s.contains("_FRAME_"),
            "Corrupted frame: {}",
            s
        );
        assert_eq!(s.len(), 17, "Frame length truncated or spliced: {}", s);
        received.push(s);
    }

    assert_eq!(received.len(), total_expected);
}

/// FR-301: Timing delay and disconnect edge behavior
#[tokio::test]
async fn test_timing_delay_and_disconnect() {
    let graph = RoutingGraph::new();
    graph
        .register_endpoint(1, 10, BackpressurePolicy::DropNewest)
        .unwrap();
    graph
        .register_endpoint(2, 10, BackpressurePolicy::DropNewest)
        .unwrap();

    // 1. Edge with fixed delay 20ms
    graph
        .add_edge(
            1,
            2,
            Some(FaultConfig {
                fixed_delay: Duration::from_millis(20),
                ..Default::default()
            }),
        )
        .unwrap();

    let start = std::time::Instant::now();
    graph.route(make_event(1, b"delayed_payload")).unwrap();

    // Event should not be immediately available
    let immediate = graph.try_recv(2).unwrap();
    assert!(immediate.is_none());

    // Await delivery
    let evt = graph.recv(2).await.unwrap();
    assert!(start.elapsed() >= Duration::from_millis(18));
    assert_eq!(&evt.payload[..], b"delayed_payload");

    // 2. Disconnect edge
    graph.remove_edge(1, 2).unwrap();
    graph
        .add_edge(
            1,
            2,
            Some(FaultConfig {
                disconnected: true,
                ..Default::default()
            }),
        )
        .unwrap();

    let dispatched = graph.route(make_event(1, b"dropped")).unwrap();
    assert_eq!(dispatched, 0);
    assert!(graph.try_recv(2).unwrap().is_none());
}
