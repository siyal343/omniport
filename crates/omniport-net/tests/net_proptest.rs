use omniport_core::PinState;
use omniport_net::rfc2217::{
    modem_state_to_pin_state, pin_state_to_control, Rfc2217Codec, Rfc2217Command,
};
use proptest::prelude::*;

proptest! {
    #[test]
    fn prop_rfc854_iac_escaping_roundtrip(data in proptest::collection::vec(any::<u8>(), 0..1024)) {
        let escaped = Rfc2217Codec::encode_data(&data);

        let mut codec = Rfc2217Codec::new();
        let (unescaped, cmds, negs) = codec.decode_chunk(&escaped);

        prop_assert_eq!(&unescaped[..], &data[..]);
        prop_assert!(cmds.is_empty());
        prop_assert!(negs.is_empty());
    }

    #[test]
    fn prop_rfc2217_baudrate_command_roundtrip(baud in 300u32..4_000_000u32) {
        let cmd = Rfc2217Command::SetBaudrate(baud);
        let bytes = Rfc2217Codec::encode_command(&cmd);

        let mut codec = Rfc2217Codec::new();
        let (data, cmds, _) = codec.decode_chunk(&bytes);

        prop_assert!(data.is_empty());
        prop_assert_eq!(cmds.len(), 1);
        prop_assert_eq!(&cmds[0], &Rfc2217Command::SetBaudrate(baud));
    }

    #[test]
    fn prop_rfc2217_modem_bits_roundtrip(modem_byte in any::<u8>()) {
        let pins = modem_state_to_pin_state(modem_byte, PinState::empty());

        // Bit 4 = CTS
        prop_assert_eq!(pins.contains(PinState::CTS), (modem_byte & 0x10) != 0);
        // Bit 5 = DSR
        prop_assert_eq!(pins.contains(PinState::DSR), (modem_byte & 0x20) != 0);
        // Bit 6 = RI
        prop_assert_eq!(pins.contains(PinState::RI), (modem_byte & 0x40) != 0);
        // Bit 7 = DCD
        prop_assert_eq!(pins.contains(PinState::DCD), (modem_byte & 0x80) != 0);
    }

    #[test]
    fn prop_pin_control_command_generation(rts in any::<bool>(), dtr in any::<bool>(), brk in any::<bool>()) {
        let mut new_pins = PinState::empty();
        if rts { new_pins |= PinState::RTS; }
        if dtr { new_pins |= PinState::DTR; }
        if brk { new_pins |= PinState::BREAK; }

        let cmds = pin_state_to_control(new_pins, PinState::empty());

        let has_rts = cmds.iter().any(|c| matches!(c, Rfc2217Command::SetControl(11)));
        let has_dtr = cmds.iter().any(|c| matches!(c, Rfc2217Command::SetControl(8)));
        let has_brk = cmds.iter().any(|c| matches!(c, Rfc2217Command::SetControl(5)));

        prop_assert_eq!(has_rts, rts);
        prop_assert_eq!(has_dtr, dtr);
        prop_assert_eq!(has_brk, brk);
    }
}
