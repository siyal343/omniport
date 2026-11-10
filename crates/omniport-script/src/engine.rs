use bytes::Bytes;
use mlua::{Function, Lua};
use omniport_core::PinState;

use crate::bindings::{HostBridge, pin_state_to_lua};
use crate::error::{Result, ScriptError};
use crate::fuel::{DEFAULT_CHECK_INTERVAL, DEFAULT_MAX_FUEL, FuelMeter};
use crate::sandbox::apply_sandbox;

pub const DEFAULT_MAX_MEMORY_BYTES: usize = 4 * 1024 * 1024;

#[derive(Debug, Clone)]
pub struct ScriptConfig {
    pub max_fuel: u64,
    pub fuel_check_interval: u32,
    pub initial_pins: PinState,
    /// Maximum Lua VM heap allocation in bytes (`0` disables the limit).
    pub max_memory_bytes: usize,
}

impl Default for ScriptConfig {
    fn default() -> Self {
        Self {
            max_fuel: DEFAULT_MAX_FUEL,
            fuel_check_interval: DEFAULT_CHECK_INTERVAL,
            initial_pins: PinState::empty(),
            max_memory_bytes: DEFAULT_MAX_MEMORY_BYTES,
        }
    }
}

pub struct ScriptEngine {
    lua: Lua,
    bridge: HostBridge,
    fuel: FuelMeter,
    max_memory_bytes: usize,
}

fn is_memory_error(err: &mlua::Error) -> bool {
    match err {
        mlua::Error::MemoryError(_) => true,
        mlua::Error::CallbackError { cause, .. }
        | mlua::Error::WithContext { cause, .. }
        | mlua::Error::BadArgument { cause, .. } => is_memory_error(cause),
        _ => false,
    }
}

fn is_fuel_error(err: &mlua::Error) -> bool {
    match err {
        mlua::Error::RuntimeError(msg) => msg.contains("FuelExceeded"),
        mlua::Error::CallbackError { cause, .. }
        | mlua::Error::WithContext { cause, .. }
        | mlua::Error::BadArgument { cause, .. } => is_fuel_error(cause),
        _ => err.to_string().contains("FuelExceeded"),
    }
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

        // 4. Enforce Lua VM memory ceiling (clamped to isize::MAX so mlua's allocator stays enabled).
        let max_memory_bytes = if config.max_memory_bytes > 0 {
            let clamped = config.max_memory_bytes.min(isize::MAX as usize);
            lua.set_memory_limit(clamped)?;
            let used = lua.used_memory();
            if used > clamped {
                return Err(ScriptError::MemoryExceeded {
                    used,
                    limit: clamped,
                });
            }
            clamped
        } else {
            0
        };

        Ok(Self {
            lua,
            bridge,
            fuel,
            max_memory_bytes,
        })
    }

    /// Loads and evaluates the Lua simulation script
    pub fn load_script(&mut self, code: &str) -> Result<()> {
        self.fuel.reset();
        if let Err(err) = self.lua.load(code).exec() {
            let mapped = if is_memory_error(&err) {
                ScriptError::MemoryExceeded {
                    used: self.lua.used_memory(),
                    limit: self.max_memory_bytes,
                }
            } else if is_fuel_error(&err) {
                ScriptError::FuelExceeded {
                    consumed: self.fuel.consumed_instructions(),
                    limit: self.fuel.max_fuel(),
                }
            } else {
                ScriptError::CompileError(err.to_string())
            };
            let _ = self.lua.gc_collect();
            let _ = self.lua.gc_collect();
            return Err(mapped);
        }

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
    /// Fuel exhaustion and memory-limit exhaustion can surface either directly
    /// from Lua (`MemoryError`) or wrapped inside Rust callbacks (`CallbackError`),
    /// so inspect the full error chain cleanly.
    fn map_callback_error(&self, callback: &'static str, err: mlua::Error) -> ScriptError {
        if is_memory_error(&err) {
            ScriptError::MemoryExceeded {
                used: self.lua.used_memory(),
                limit: self.max_memory_bytes,
            }
        } else if is_fuel_error(&err) {
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
    /// discarded and unreachable Lua memory is reclaimed before the next call.
    fn finish_callback(
        &mut self,
        result: mlua::Result<()>,
        callback: &'static str,
    ) -> Result<Vec<Bytes>> {
        match result {
            Ok(()) => Ok(self.bridge.drain_outbound()),
            Err(e) => {
                self.bridge.clear_outbound();
                let mapped = self.map_callback_error(callback, e);
                let _ = self.lua.gc_collect();
                let _ = self.lua.gc_collect();
                Err(mapped)
            }
        }
    }

    /// Dispatches an incoming serial packet to the script's `on_rx(payload, pins)` callback
    pub fn dispatch_rx(&mut self, payload: &[u8], pins: PinState) -> Result<Vec<Bytes>> {
        let globals = self.lua.globals();
        if let Ok(on_rx) = globals.get::<Function>("on_rx") {
            self.fuel.reset();

            let lua_payload = self
                .lua
                .create_string(payload)
                .map_err(|e| self.map_callback_error("on_rx", e))?;
            let lua_pins = pin_state_to_lua(&self.lua, pins)
                .map_err(|e| self.map_callback_error("on_rx", e))?;

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
            let lua_pins = pin_state_to_lua(&self.lua, new_pins)
                .map_err(|e| self.map_callback_error("on_pin_change", e))?;

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

    pub fn used_memory(&self) -> usize {
        self.lua.used_memory()
    }

    pub fn max_memory_bytes(&self) -> usize {
        self.max_memory_bytes
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

    #[test]
    fn test_string_rep_memory_bomb_is_stopped_and_engine_recovers() {
        let config = ScriptConfig {
            max_memory_bytes: 512 * 1024,
            ..Default::default()
        };
        let mut engine = ScriptEngine::new(config).unwrap();
        engine
            .load_script(
                r#"
                function on_rx(payload, pins)
                    if payload == "bomb" then
                        serial.send("PARTIAL")
                        local huge = string.rep("A", 2 * 1024 * 1024)
                        serial.send(huge)
                        return
                    end
                    serial.send("ALIVE:" .. payload)
                end
                "#,
            )
            .unwrap();

        let err = engine.dispatch_rx(b"bomb", PinState::empty()).unwrap_err();
        assert!(
            matches!(err, ScriptError::MemoryExceeded { limit: 524_288, .. }),
            "expected MemoryExceeded for string.rep OOM, got {err:?}"
        );

        // Next valid dispatch must run cleanly without stale outbound leakage or broken VM.
        let out = engine.dispatch_rx(b"ok", PinState::empty()).unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(&out[0][..], b"ALIVE:ok");
    }

    #[test]
    fn test_table_concat_memory_bomb_is_stopped() {
        let config = ScriptConfig {
            max_memory_bytes: 512 * 1024,
            ..Default::default()
        };
        let mut engine = ScriptEngine::new(config).unwrap();
        engine
            .load_script(
                r#"
                function on_rx(payload, pins)
                    local chunk = string.rep("X", 64 * 1024)
                    local t = {}
                    for i = 1, 16 do
                        t[i] = chunk
                    end
                    return table.concat(t)
                end
                "#,
            )
            .unwrap();

        let err = engine.dispatch_rx(b"go", PinState::empty()).unwrap_err();
        assert!(
            matches!(err, ScriptError::MemoryExceeded { .. }),
            "expected MemoryExceeded for table.concat OOM, got {err:?}"
        );
    }

    #[test]
    fn test_top_level_script_allocation_respects_memory_limit() {
        let config = ScriptConfig {
            max_memory_bytes: 256 * 1024,
            ..Default::default()
        };
        let mut engine = ScriptEngine::new(config).unwrap();
        let err = engine
            .load_script("local huge = string.rep('Z', 1024 * 1024)")
            .unwrap_err();
        assert!(
            matches!(err, ScriptError::MemoryExceeded { .. }),
            "expected top-level MemoryExceeded, got {err:?}"
        );
    }

    #[test]
    fn test_oversized_serial_send_string_and_table_are_rejected_safely() {
        let config = ScriptConfig {
            max_fuel: 50_000_000,
            max_memory_bytes: 32 * 1024 * 1024,
            ..Default::default()
        };
        let mut engine = ScriptEngine::new(config).unwrap();
        engine
            .load_script(
                r#"
                function on_rx(payload, pins)
                    if payload == "str" then
                        serial.send(string.rep("A", 1024 * 1024 + 1))
                    elseif payload == "tbl" then
                        local t = {}
                        for i = 1, (1024 * 1024 + 1) do
                            t[i] = 0x41
                        end
                        serial.send(t)
                    end
                end
                "#,
            )
            .unwrap();

        let err_str = engine.dispatch_rx(b"str", PinState::empty()).unwrap_err();
        assert!(
            matches!(err_str, ScriptError::MemoryExceeded { .. }),
            "expected MemoryExceeded for oversized serial.send string, got {err_str:?}"
        );

        let err_tbl = engine.dispatch_rx(b"tbl", PinState::empty()).unwrap_err();
        assert!(
            matches!(err_tbl, ScriptError::MemoryExceeded { .. }),
            "expected MemoryExceeded for oversized serial.send table, got {err_tbl:?}"
        );
    }
}
