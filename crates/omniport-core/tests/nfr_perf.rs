//! NFR-101 / NFR-102 performance harness.
//!
//! Marked `#[ignore]` so the default (debug) workspace suite stays fast; the
//! `perf` CI job runs these in release mode with `--ignored --nocapture` and
//! enforces the thresholds below. Every failure prints the measured value next
//! to the target.

use bytes::Bytes;
use omniport_core::routing::{BackpressurePolicy, RoutingGraph};
use omniport_core::{FrameFlags, PinState, SerialEvent};
use std::time::Instant;

fn event(source: u16, payload: &[u8]) -> SerialEvent {
    SerialEvent {
        timestamp_ns: 1_000_000,
        source_id: source,
        flags: FrameFlags::empty(),
        pins: PinState::empty(),
        payload: Bytes::copy_from_slice(payload),
    }
}

fn percentile(sorted: &[u128], percentile: f64) -> u128 {
    let rank = ((sorted.len() as f64 - 1.0) * percentile).round() as usize;
    sorted[rank.min(sorted.len() - 1)]
}

fn microseconds(nanos: u128) -> f64 {
    nanos as f64 / 1_000.0
}

/// NFR-101: local virtual-pair forwarding p99 < 500 microseconds for 64-byte
/// frames. Measures the full route -> receive hop (queue enqueue and dequeue).
#[test]
#[ignore = "performance harness; run by the perf CI job in release mode"]
fn test_nfr101_loopback_pair_forwarding_latency() {
    const TARGET_P99_US: f64 = 500.0;
    const ITERATIONS: usize = 20_000;

    let graph = RoutingGraph::new();
    graph
        .register_endpoint(1, 64, BackpressurePolicy::DropNewest)
        .expect("register source endpoint");
    graph
        .register_endpoint(2, 64, BackpressurePolicy::DropNewest)
        .expect("register destination endpoint");
    graph.add_edge(1, 2, None).expect("add loopback edge");
    let base = event(1, &[0x5A_u8; 64]);

    // Warm up caches / page tables before measuring.
    for _ in 0..1_000 {
        assert_eq!(graph.route(base.clone()).expect("route warmup"), 1);
        let _ = graph.try_recv(2).expect("recv warmup");
    }

    let mut samples = Vec::with_capacity(ITERATIONS);
    for _ in 0..ITERATIONS {
        let start = Instant::now();
        let dispatched = graph.route(base.clone()).expect("route");
        assert_eq!(dispatched, 1, "loopback edge must dispatch once");
        let received = graph.try_recv(2).expect("recv").expect("queued frame");
        let elapsed = start.elapsed().as_nanos();
        assert_eq!(received.payload.len(), 64);
        samples.push(elapsed);
    }
    samples.sort_unstable();

    let p50 = microseconds(percentile(&samples, 0.50));
    let p99 = microseconds(percentile(&samples, 0.99));
    let max = microseconds(*samples.last().expect("samples"));
    println!(
        "NFR-101: loopback pair forwarding, 64-byte frames, {ITERATIONS} samples — \
         p50={p50:.3} us, p99={p99:.3} us, max={max:.3} us (target p99 < {TARGET_P99_US:.1} us)"
    );
    assert!(
        p99 < TARGET_P99_US,
        "NFR-101 violated: p99 {p99:.3} us exceeds target {TARGET_P99_US:.1} us \
         (p50 {p50:.3} us, max {max:.3} us)"
    );
}

/// NFR-102: aggregate throughput >= 12 Mbps across 32 active port pairs with
/// zero dropped bytes. Each pair is a directed source -> destination edge; the
/// receiver is drained every iteration so the assertion also proves the
/// bounded queues never back-pressure (zero drops).
#[test]
#[ignore = "performance harness; run by the perf CI job in release mode"]
fn test_nfr102_throughput_32_pairs_zero_drops() {
    const TARGET_MBPS: f64 = 12.0;
    const PAIRS: u16 = 32;
    const FRAMES_PER_PAIR: usize = 2_000;
    const PAYLOAD_BYTES: usize = 1_024;

    let graph = RoutingGraph::new();
    let mut events_by_pair = Vec::with_capacity(PAIRS as usize);
    for pair in 0..PAIRS {
        let source = pair * 2 + 1;
        let destination = source + 1;
        graph
            .register_endpoint(source, 256, BackpressurePolicy::DropNewest)
            .expect("register source");
        graph
            .register_endpoint(destination, 256, BackpressurePolicy::DropNewest)
            .expect("register destination");
        graph
            .add_edge(source, destination, None)
            .expect("add directed pair edge");

        let mut sample = event(source, &vec![0xA5_u8; PAYLOAD_BYTES]);
        sample.timestamp_ns = pair as u64;
        events_by_pair.push(sample);
    }

    let mut sent_bytes = 0_u64;
    let mut received_bytes = 0_u64;
    let mut received_frames = 0_u64;
    let started = Instant::now();
    for _ in 0..FRAMES_PER_PAIR {
        for (pair, sample) in events_by_pair.iter().enumerate() {
            let destination = pair as u16 * 2 + 2;
            assert_eq!(
                graph.route(sample.clone()).expect("route frame"),
                1,
                "each pair must dispatch to exactly its destination"
            );
            sent_bytes += PAYLOAD_BYTES as u64;

            let received = graph
                .try_recv(destination)
                .expect("recv")
                .expect("frame must be queued");
            assert_eq!(received.payload.len(), PAYLOAD_BYTES);
            received_bytes += received.payload.len() as u64;
            received_frames += 1;
        }
    }
    let elapsed = started.elapsed();
    for pair in 0..PAIRS {
        assert_eq!(
            graph.queue_len(pair * 2 + 2).expect("queue len"),
            0,
            "destination queue should be drained"
        );
    }

    let expected_frames = FRAMES_PER_PAIR as u64 * PAIRS as u64;
    let mbps = (received_bytes as f64 * 8.0) / elapsed.as_secs_f64() / 1_000_000.0;
    println!(
        "NFR-102: aggregate throughput across {PAIRS} pairs — {mbps:.1} Mbps \
         ({received_bytes} bytes in {:.3} ms, {received_frames} frames, 0 dropped) \
         (target >= {TARGET_MBPS:.1} Mbps)",
        elapsed.as_secs_f64() * 1_000.0
    );
    assert_eq!(
        sent_bytes, received_bytes,
        "NFR-102 violated: sent {sent_bytes} bytes but received {received_bytes}"
    );
    assert_eq!(
        received_frames, expected_frames,
        "NFR-102 violated: expected {expected_frames} frames, got {received_frames}"
    );
    assert!(
        mbps >= TARGET_MBPS,
        "NFR-102 violated: measured {mbps:.1} Mbps, target >= {TARGET_MBPS:.1} Mbps"
    );
}
