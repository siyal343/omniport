use omniport_core::SerialConfig;
#[cfg(not(unix))]
use omniport_core::{AsyncEndpoint, FrameFlags, PinState, SerialEvent};
use omniport_hal_posix::{PtyEndpoint, SymlinkManager, TokenBucketThrottler, calculate_byte_rate};
use proptest::prelude::*;
#[cfg(not(unix))]
use tokio::io::{AsyncReadExt, AsyncWriteExt};

proptest! {
    #![proptest_config(ProptestConfig { failure_persistence: None, ..ProptestConfig::default() })]

    #[test]
    fn prop_baud_rate_formula(
        baud_rate in 300u32..4_000_000u32,
        data_bits in 5u8..=8u8,
        stop_bits in 1u8..=2u8,
        parity in 0u8..=2u8,
    ) {
        let cfg = SerialConfig {
            baud_rate,
            data_bits,
            stop_bits,
            parity,
        };

        let calculated = calculate_byte_rate(&cfg);

        let p_bits = if parity != 0 { 1.0 } else { 0.0 };
        let total_bits_per_char = 1.0 + (data_bits as f64) + p_bits + (stop_bits as f64);
        let expected = (baud_rate as f64) / total_bits_per_char;

        prop_assert!((calculated - expected).abs() < 1e-6);

        let throttler = TokenBucketThrottler::new(cfg);
        prop_assert!((throttler.byte_rate() - expected).abs() < 1e-6);
    }

    #[test]
    fn prop_token_bucket_delay_accuracy(
        byte_count in 1usize..5000usize,
        baud in 1200u32..115_200u32,
    ) {
        let cfg = SerialConfig {
            baud_rate: baud,
            data_bits: 8,
            stop_bits: 1,
            parity: 0,
        };
        let mut throttler = TokenBucketThrottler::new(cfg);

        // Exhaust starting tokens
        let start_time_ns = 1_000_000_000u64;
        let initial_cap = throttler.capacity().ceil() as usize;
        let _ = throttler.throttle(initial_cap, start_time_ns);

        // Throttle byte_count with zero initial tokens
        let delay = throttler.throttle(byte_count, start_time_ns);
        let expected_secs = (byte_count as f64) / throttler.byte_rate();
        let expected_ns = (expected_secs * 1_000_000_000.0) as u64;

        // Verify delay within 1 microsecond (1,000 ns) tolerance
        let diff_ns = (delay.as_nanos() as u64).abs_diff(expected_ns);
        prop_assert!(diff_ns <= 1_000, "delay_ns={} expected_ns={} diff={}", delay.as_nanos(), expected_ns, diff_ns);
    }

    #[test]
    fn prop_symlink_guard_lifecycle(
        subname in "[a-z0-9_]{4,12}"
    ) {
        let temp_dir = tempfile::tempdir().unwrap();
        let target = temp_dir.path().join(format!("target_{}", subname));
        let link = temp_dir.path().join(format!("link_{}", subname));

        std::fs::write(&target, b"device_node").unwrap();

        {
            let guard = SymlinkManager::create(&target, &link).unwrap();
            prop_assert!(link.exists());
            prop_assert_eq!(guard.target(), target.as_path());
            prop_assert_eq!(guard.link_path(), link.as_path());
        }

        prop_assert!(!link.exists());
        prop_assert!(target.exists());
    }
}

#[tokio::test]
async fn test_pty_endpoint_stream_roundtrip_proptest() {
    let rt = tokio::runtime::Handle::current();
    let _guard = rt.enter();

    let cfg = SerialConfig {
        baud_rate: 115_200,
        data_bits: 8,
        stop_bits: 1,
        parity: 0,
    };

    #[cfg_attr(unix, allow(unused_mut))]
    let mut pty = PtyEndpoint::open(10, "test_vport", cfg, None).expect("open pty");

    #[cfg(not(unix))]
    {
        let mut slave = pty.take_mock_slave().expect("take slave");

        // Transmit packets from slave to pty (recv)
        for i in 0..10 {
            let msg = format!("PAYLOAD_PACKET_{}\n", i);
            slave.write_all(msg.as_bytes()).await.expect("slave write");

            let event = pty.recv().await.expect("pty recv");
            assert_eq!(event.source_id, 10);
            assert_eq!(&event.payload[..], msg.as_bytes());
        }

        // Transmit packets from pty to slave (send)
        for i in 0..10 {
            let msg = format!("REPLY_PACKET_{}\n", i);
            let send_event = SerialEvent {
                timestamp_ns: 2000 + i,
                source_id: 10,
                flags: FrameFlags::empty(),
                pins: PinState::empty(),
                payload: bytes::Bytes::copy_from_slice(msg.as_bytes()),
            };
            pty.send(send_event).await.expect("pty send");

            let mut buf = vec![0u8; msg.len()];
            slave.read_exact(&mut buf).await.expect("slave read");
            assert_eq!(&buf[..], msg.as_bytes());
        }
    }
}
