use bytes::Bytes;
use omniport_core::{AsyncEndpoint, FrameFlags, PinState, SerialConfig, SerialEvent};
use omniport_hal_windows::{Com0ComController, Win32SerialEndpoint, format_com_path};
use proptest::prelude::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

proptest! {
    #![proptest_config(ProptestConfig { failure_persistence: None, ..ProptestConfig::default() })]

    #[test]
    fn prop_com_path_formatting(
        port_num in 1u32..500u32,
    ) {
        let raw_name = format!("COM{}", port_num);
        let formatted = format_com_path(&raw_name);
        prop_assert_eq!(&formatted, &format!(r"\\.\COM{}", port_num));

        // Re-formatting an already prefixed path should be idempotent
        let re_formatted = format_com_path(&formatted);
        prop_assert_eq!(&re_formatted, &formatted);
    }

    #[test]
    fn prop_virtual_pair_isolation_and_raii(
        start_base in 50u32..2000u32,
    ) {
        let controller = Com0ComController::new();
        let (port_a, port_b) = controller.find_free_pair_names(start_base).unwrap();

        prop_assert_ne!(&port_a, &port_b);

        {
            let guard = controller.create_pair(&port_a, &port_b).unwrap();
            prop_assert_eq!(guard.port_a(), port_a.as_str());
            prop_assert_eq!(guard.port_b(), port_b.as_str());

            let list = controller.list_pairs().unwrap();
            prop_assert_eq!(list.len(), 1);
            prop_assert_eq!(&list[0].port_a, &port_a);
            prop_assert_eq!(&list[0].port_b, &port_b);
        }

        // Dropping guard must remove the pair
        let list_after = controller.list_pairs().unwrap();
        prop_assert!(list_after.is_empty());
    }
}

#[tokio::test]
async fn test_uart_endpoint_stream_proptest() {
    let cfg = SerialConfig {
        baud_rate: 115_200,
        data_bits: 8,
        stop_bits: 1,
        parity: 0,
    };

    let mut uart =
        Win32SerialEndpoint::open_mock(100, "vcom_prop", "COM100", cfg).expect("open mock uart");

    let mut peer = uart.take_mock_peer().expect("take peer");

    for i in 0..15 {
        let msg = format!("WIN32_UART_PAYLOAD_{}\r\n", i);

        // 1. Send from UART to Peer
        let send_evt = SerialEvent {
            timestamp_ns: 5000 + i,
            source_id: 100,
            flags: FrameFlags::empty(),
            pins: PinState::RTS | PinState::DTR,
            payload: Bytes::copy_from_slice(msg.as_bytes()),
        };
        uart.send(send_evt).await.expect("send to peer");

        let mut read_buf = vec![0u8; msg.len()];
        peer.read_exact(&mut read_buf).await.expect("peer read");
        assert_eq!(&read_buf[..], msg.as_bytes());

        // 2. Send from Peer to UART (recv)
        peer.write_all(msg.as_bytes()).await.expect("peer write");
        let recv_evt = uart.recv().await.expect("uart recv");
        assert_eq!(recv_evt.source_id, 100);
        assert_eq!(&recv_evt.payload[..], msg.as_bytes());
    }
}
