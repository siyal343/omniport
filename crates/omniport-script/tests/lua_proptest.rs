use omniport_core::PinState;
use omniport_script::{ScriptConfig, ScriptEngine, ScriptError};
use proptest::prelude::*;

proptest! {
    #[test]
    fn prop_lua_binary_payload_preservation(payload in proptest::collection::vec(any::<u8>(), 0..512)) {
        let mut engine = ScriptEngine::new(ScriptConfig::default()).unwrap();
        let script = r#"
            function on_rx(data, pins)
                -- Echo exact raw binary bytes back
                serial.send(data)
            end
        "#;
        engine.load_script(script).unwrap();

        let responses = engine.dispatch_rx(&payload, PinState::empty()).unwrap();
        prop_assert_eq!(responses.len(), 1);
        prop_assert_eq!(&responses[0][..], &payload[..]);
    }

    #[test]
    fn prop_lua_pin_states_roundtrip(rts in any::<bool>(), cts in any::<bool>(), dtr in any::<bool>(), dcd in any::<bool>()) {
        let mut initial_pins = PinState::empty();
        if rts { initial_pins |= PinState::RTS; }
        if cts { initial_pins |= PinState::CTS; }
        if dtr { initial_pins |= PinState::DTR; }
        if dcd { initial_pins |= PinState::DCD; }

        let config = ScriptConfig {
            initial_pins,
            ..Default::default()
        };
        let mut engine = ScriptEngine::new(config).unwrap();

        let script = r#"
            function on_rx(data, pins)
                -- Read pins from Lua, toggle RTS
                local p = serial.get_pins()
                p.rts = not p.rts
                serial.set_pins(p)
            end
        "#;
        engine.load_script(script).unwrap();

        engine.dispatch_rx(b"", initial_pins).unwrap();
        let new_pins = engine.current_pins();

        prop_assert_eq!(new_pins.contains(PinState::RTS), !rts);
        prop_assert_eq!(new_pins.contains(PinState::CTS), cts);
        prop_assert_eq!(new_pins.contains(PinState::DTR), dtr);
        prop_assert_eq!(new_pins.contains(PinState::DCD), dcd);
    }

    #[test]
    fn prop_fuel_exhaustion_guarantee(iterations in 500u32..5_000u32) {
        let max_fuel = 2_000;
        let config = ScriptConfig {
            max_fuel,
            fuel_check_interval: 20,
            ..Default::default()
        };
        let mut engine = ScriptEngine::new(config).unwrap();

        let script = format!(
            r#"
                function on_rx(data, pins)
                    local s = 0
                    for i = 1, {} do
                        s = s + i
                    end
                    serial.send(tostring(s))
                end
            "#,
            iterations
        );
        engine.load_script(&script).unwrap();

        let result = engine.dispatch_rx(b"", PinState::empty());
        if iterations > 3_000 {
            prop_assert!(result.is_err());
            match result.unwrap_err() {
                ScriptError::FuelExceeded { consumed, limit } => {
                    prop_assert!(consumed >= max_fuel);
                    prop_assert_eq!(limit, max_fuel);
                }
                other => panic!("Expected FuelExceeded, got: {:?}", other),
            }
        }
    }
}
