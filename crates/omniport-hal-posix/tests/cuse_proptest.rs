use omniport_core::{PinState, SerialConfig};
use omniport_hal_posix::{
    SerialIoctlHandler, TIOCM_CAR, TIOCM_CTS, TIOCM_DSR, TIOCM_DTR, TIOCM_RNG, TIOCM_RTS, TIOCMBIC,
    TIOCMBIS, pin_state_to_tiocm, tiocm_to_pin_state,
};
use proptest::prelude::*;
use std::time::Duration;

const VALID_MODEM_MASK: u32 = TIOCM_DTR | TIOCM_RTS | TIOCM_CTS | TIOCM_CAR | TIOCM_RNG | TIOCM_DSR;

proptest! {
    #![proptest_config(ProptestConfig { failure_persistence: None, ..ProptestConfig::default() })]

    #[test]
    fn prop_pin_state_tiocm_roundtrip(
        rts in any::<bool>(),
        cts in any::<bool>(),
        dtr in any::<bool>(),
        dsr in any::<bool>(),
        dcd in any::<bool>(),
        ri in any::<bool>(),
    ) {
        let mut pins = PinState::empty();
        if rts { pins |= PinState::RTS; }
        if cts { pins |= PinState::CTS; }
        if dtr { pins |= PinState::DTR; }
        if dsr { pins |= PinState::DSR; }
        if dcd { pins |= PinState::DCD; }
        if ri { pins |= PinState::RI; }

        let tiocm = pin_state_to_tiocm(pins);
        let recovered = tiocm_to_pin_state(tiocm);
        prop_assert_eq!(recovered, pins);
    }

    #[test]
    fn prop_tiocm_bits_roundtrip(
        raw_bits in 0u32..=0x1FFu32,
    ) {
        let valid_bits = raw_bits & VALID_MODEM_MASK;
        let pins = tiocm_to_pin_state(valid_bits);
        let recovered_bits = pin_state_to_tiocm(pins);
        prop_assert_eq!(recovered_bits, valid_bits);
    }

    #[test]
    fn prop_cuse_ioctl_bis_bic_invariants(
        initial_raw in 0u32..=0x1FFu32,
        bis_raw in 0u32..=0x1FFu32,
        bic_raw in 0u32..=0x1FFu32,
    ) {
        let initial_bits = initial_raw & VALID_MODEM_MASK;
        let bis_bits = bis_raw & VALID_MODEM_MASK;
        let bic_bits = bic_raw & VALID_MODEM_MASK;

        let mut handler = SerialIoctlHandler::new(
            tiocm_to_pin_state(initial_bits),
            SerialConfig::default(),
        );

        // Apply TIOCMBIS
        handler.handle_ioctl(TIOCMBIS, &bis_bits.to_ne_bytes()).unwrap();
        let after_bis = pin_state_to_tiocm(handler.pins());
        prop_assert_eq!(after_bis, initial_bits | bis_bits);

        // Apply TIOCMBIC
        handler.handle_ioctl(TIOCMBIC, &bic_bits.to_ne_bytes()).unwrap();
        let after_bic = pin_state_to_tiocm(handler.pins());
        prop_assert_eq!(after_bic, (initial_bits | bis_bits) & !bic_bits);
    }

    #[test]
    fn prop_cuse_transition_counters(
        transitions in prop::collection::vec(0u32..=0x1FFu32, 1..30),
    ) {
        let mut handler = SerialIoctlHandler::new(PinState::empty(), SerialConfig::default());
        let mut expected_cts = 0u32;
        let mut expected_dsr = 0u32;
        let mut expected_dcd = 0u32;
        let mut expected_ri = 0u32;

        let mut prev = PinState::empty();

        for raw in transitions {
            let next = tiocm_to_pin_state(raw & VALID_MODEM_MASK);
            if prev.contains(PinState::CTS) != next.contains(PinState::CTS) {
                expected_cts += 1;
            }
            if prev.contains(PinState::DSR) != next.contains(PinState::DSR) {
                expected_dsr += 1;
            }
            if prev.contains(PinState::DCD) != next.contains(PinState::DCD) {
                expected_dcd += 1;
            }
            if prev.contains(PinState::RI) != next.contains(PinState::RI) {
                expected_ri += 1;
            }

            handler.set_pins(next);
            prev = next;
        }

        let counters = handler.counters();
        prop_assert_eq!(counters.cts, expected_cts);
        prop_assert_eq!(counters.dsr, expected_dsr);
        prop_assert_eq!(counters.dcd, expected_dcd);
        prop_assert_eq!(counters.rng, expected_ri);
    }
}

#[tokio::test]
async fn test_cuse_tiocmiwait_timeout() {
    let handler = SerialIoctlHandler::new(PinState::empty(), SerialConfig::default());

    // Await pin change on CTS with a 50 ms timeout without firing any changes
    let res = handler
        .wait_for_pin_change(TIOCM_CTS, Some(Duration::from_millis(50)))
        .await;

    assert!(res.is_err());
    let err_str = res.err().unwrap().to_string();
    assert!(err_str.contains("timed out") || err_str.contains("TimedOut"));
}
