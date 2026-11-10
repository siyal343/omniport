use bytes::Bytes;
use mlua::{Function, Lua};
use omniport_core::PinState;

use crate::bindings::{HostBridge, pin_state_to_lua};
use crate::error::{Result, ScriptError};
use crate::fuel::{DEFAULT_CHECK_INTERVAL, DEFAULT_MAX_FUEL, FuelMeter};
use crate::sandbox::apply_sandbox;

#[derive(Debug, Clone)]
pub struct ScriptConfig {
    pub max_fuel: u64,
    pub fuel_check_interval: u32,
    pub initial_pins: PinState,
}

impl Default for ScriptConfig {
    fn default() -> Self {
        Self {
            max_fuel: DEFAULT_MAX_FUEL,
            fuel_check_interval: DEFAULT_CHECK_INTERVAL,
            initial_pins: PinState::empty(),
        }
    }
}

pub struct ScriptEngine {
    lua: Lua,
    bridge: HostBridge,
    fuel: FuelMeter,
}

impl ScriptEngine {
    pub fn new(config: ScriptConfig) -> Result<Self> {
        let lua = Lua::new();

        // 1. Enforce strict sandboxing
        apply_sandbox(&lua)?;

        // 2. Setup host bindings for serial communication and pin states
        let bridge = HostBridge::new();
        bridge.set_pins(config.initial_pins);
        bridge.register(&lua)?;

        // 3. Attach instruction counter fuel hook
        let fuel = FuelMeter::new(config.max_fuel, config.fuel_check_interval);
        fuel.attach_hook(&lua)?;

        Ok(Self { lua, bridge, fuel })
    }

    /// Loads and evaluates the Lua simulation script
    pub fn load_script(&mut self, code: &str) -> Result<()> {
        self.fuel.reset();
        self.lua
            .load(code)
            .exec()
            .map_err(|e| ScriptError::CompileError(e.to_string()))?;

        // If the script defines on_init(), call it
        let globals = self.lua.globals();
        if let Ok(on_init) = globals.get::<Function>("on_init") {
            self.fuel.reset();
            let result = on_init.call::<()>(());
            self.finish_callback(result, "on_init")?;
        }

        Ok(())
    }

    /// Converts an `mlua` error into the precise `ScriptError` variant.
    ///
    /// Fuel exhaustion is raised from the instruction hook as a runtime error;
    /// it must be surfaced as `FuelExceeded` for every callback (including
    /// `on_init`, `on_timer` and `on_pin_change`), not just `on_rx`.
    fn map_callback_error(&self, callback: &'static str, err: mlua::Error) -> ScriptError {
        if err.to_string().contains("FuelExceeded") {
            ScriptError::FuelExceeded {
                consumed: self.fuel.consumed_instructions(),
                limit: self.fuel.max_fuel(),
            }
        } else {
            ScriptError::CallbackError {
                callback: callback.to_string(),
                reason: err.to_string(),
            }
        }
    }

    /// Drains the output queue after a successful callback.
    ///
    /// If the callback failed part-way through, any bytes it already queued are
    /// discarded: otherwise a half-written frame would leak into the next
    /// dispatch (stale/partial writes on the wire).
    fn finish_callback(
        &mut self,
        result: mlua::Result<()>,
        callback: &'static str,
    ) -> Result<Vec<Bytes>> {
        match result {
            Ok(()) => Ok(self.bridge.drain_outbound()),
            Err(e) => {
                self.bridge.clear_outbound();
                Err(self.map_callback_error(callback, e))
            }
        }
    }

    /// Dispatches an incoming serial packet to the script's `on_rx(payload, pins)` callback
    pub fn dispatch_rx(&mut self, payload: &[u8], pins: PinState) -> Result<Vec<Bytes>> {
        let globals = self.lua.globals();
        if let Ok(on_rx) = globals.get::<Function>("on_rx") {
            self.fuel.reset();

            let lua_payload = self.lua.create_string(payload)?;
            let lua_pins = pin_state_to_lua(&self.lua, pins)?;

            let result = on_rx.call::<()>((lua_payload, lua_pins));
            self.finish_callback(result, "on_rx")
        } else {
            Ok(self.bridge.drain_outbound())
        }
    }

    /// Dispatches an input pin change event to the script's `on_pin_change(pins)` callback
    pub fn dispatch_pin_change(&mut self, new_pins: PinState) -> Result<Vec<Bytes>> {
        let globals = self.lua.globals();
        if let Ok(on_pin_change) = globals.get::<Function>("on_pin_change") {
            self.fuel.reset();
            let lua_pins = pin_state_to_lua(&self.lua, new_pins)?;

            let result = on_pin_change.call::<()>(lua_pins);
            self.finish_callback(result, "on_pin_change")
        } else {
            Ok(self.bridge.drain_outbound())
        }
    }

    /// Dispatches a periodic timer event to the script's `on_timer(timer_id)` callback
    pub fn dispatch_timer(&mut self, timer_id: u32) -> Result<Vec<Bytes>> {
        let globals = self.lua.globals();
        if let Ok(on_timer) = globals.get::<Function>("on_timer") {
            self.fuel.reset();
            let result = on_timer.call::<()>(timer_id);
            self.finish_callback(result, "on_timer")
        } else {
            Ok(self.bridge.drain_outbound())
        }
    }

    pub fn current_pins(&self) -> PinState {
        self.bridge.get_pins()
    }

    pub fn set_pins(&mut self, pins: PinState) {
        self.bridge.set_pins(pins);
    }

    pub fn drain_logs(&mut self) -> Vec<String> {
        self.bridge.drain_logs()
    }

    pub fn consumed_fuel(&self) -> u64 {
        self.fuel.consumed_instructions()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_echo_simulator_script() {
        let mut engine = ScriptEngine::new(ScriptConfig::default()).unwrap();

        let script = r#"
            function on_rx(payload, pins)
                -- Echo back reversed string with prefix
                local reply = "ACK:" .. string.reverse(payload)
                serial.send(reply)
                serial.set_pins({ rts = true, cts = true })
            end
        "#;

        engine.load_script(script).unwrap();

        let responses = engine.dispatch_rx(b"PING", PinState::empty()).unwrap();
        assert_eq!(responses.len(), 1);
        assert_eq!(&responses[0][..], b"ACK:GNIP");

        let pins = engine.current_pins();
        assert!(pins.contains(PinState::RTS));
        assert!(pins.contains(PinState::CTS));
    }

    #[test]
    fn test_stateful_counter_simulator() {
        let mut engine = ScriptEngine::new(ScriptConfig::default()).unwrap();

        let script = r#"
            local counter = 0
            function on_timer(timer_id)
                counter = counter + 1
                serial.send(string.format("$TICK,%d,%d\r\n", timer_id, counter))
            end
        "#;

        engine.load_script(script).unwrap();

        let r1 = engine.dispatch_timer(1).unwrap();
        assert_eq!(&r1[0][..], b"$TICK,1,1\r\n");

        let r2 = engine.dispatch_timer(1).unwrap();
        assert_eq!(&r2[0][..], b"$TICK,1,2\r\n");
    }

    #[test]
    fn test_fuel_exhaustion_in_on_rx_aborts_cleanly() {
        let config = ScriptConfig {
            max_fuel: 5_000,
            ..Default::default()
        };
        let mut engine = ScriptEngine::new(config).unwrap();

        let runaway_script = r#"
            function on_rx(payload, pins)
                while true do end
            end
        "#;

        engine.load_script(runaway_script).unwrap();
        let err = engine.dispatch_rx(b"DOOM", PinState::empty()).unwrap_err();

        match err {
            ScriptError::FuelExceeded { consumed, limit } => {
                assert!(consumed >= 5_000);
                assert_eq!(limit, 5_000);
            }
            other => panic!("Expected FuelExceeded, got: {:?}", other),
        }
    }

    #[test]
    fn test_fuel_exhaustion_in_on_timer_reports_fuel_exceeded() {
        let config = ScriptConfig {
            max_fuel: 5_000,
            ..Default::default()
        };
        let mut engine = ScriptEngine::new(config).unwrap();

        engine
            .load_script("function on_timer(id) while true do end end")
            .unwrap();
        let err = engine.dispatch_timer(1).unwrap_err();

        match err {
            ScriptError::FuelExceeded { consumed, limit } => {
                assert!(consumed >= 5_000);
                assert_eq!(limit, 5_000);
            }
            other => panic!("Expected FuelExceeded, got: {:?}", other),
        }
    }

    #[test]
    fn test_failed_callback_does_not_leak_partial_outbound() {
        // A script that sends a byte and then errors must not leak that
        // half-written frame into the next dispatch.
        let mut engine = ScriptEngine::new(ScriptConfig::default()).unwrap();
        engine
            .load_script(
                r#"
                function on_rx(payload, pins)
                    if payload == "boom" then
                        serial.send("STALE")
                        error("kaboom")
                    end
                    serial.send("OK")
                end
                "#,
            )
            .unwrap();

        let err = engine.dispatch_rx(b"boom", PinState::empty()).unwrap_err();
        assert!(matches!(err, ScriptError::CallbackError { .. }));

        let responses = engine.dispatch_rx(b"good", PinState::empty()).unwrap();
        assert_eq!(responses.len(), 1, "stale output leaked: {responses:?}");
        assert_eq!(&responses[0][..], b"OK");
    }

    #[test]
    fn test_bad_pin_state_type_is_rejected_without_crashing() {
        // A Lua script calling serial.set_pins with a non-boolean value must
        // yield a clean runtime error rather than aborting the host process.
        let mut engine = ScriptEngine::new(ScriptConfig::default()).unwrap();
        engine
            .load_script(
                r#"
                function on_rx(payload, pins)
                    serial.set_pins({ rts = "not-a-bool" })
                end
                "#,
            )
            .unwrap();
        let err = engine.dispatch_rx(b"x", PinState::empty()).unwrap_err();
        assert!(
            matches!(err, ScriptError::CallbackError { ref callback, .. } if callback == "on_rx"),
            "unexpected error: {err:?}"
        );
    }
}
