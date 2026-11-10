use bytes::Bytes;
use mlua::{Lua, Result, Table, Value};
use omniport_core::PinState;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex};

/// Hard per-frame cap on `serial.send(...)` payloads to bound Rust-side allocations
/// (especially sparse or giant Lua tables whose `raw_len()` bypasses `mlua`'s heap limit).
pub const MAX_SERIAL_SEND_BYTES: usize = 1024 * 1024;
const MAX_OUTBOUND_QUEUE_BYTES: usize = 16 * 1024 * 1024;

#[derive(Clone, Default)]
pub struct HostBridge {
    outbound_queue: Arc<Mutex<Vec<Bytes>>>,
    pins_state: Arc<AtomicU8>,
    log_messages: Arc<Mutex<Vec<String>>>,
}

impl HostBridge {
    pub fn new() -> Self {
        Self {
            outbound_queue: Arc::new(Mutex::new(Vec::new())),
            pins_state: Arc::new(AtomicU8::new(PinState::empty().bits())),
            log_messages: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub fn set_pins(&self, pins: PinState) {
        self.pins_state.store(pins.bits(), Ordering::Release);
    }

    pub fn get_pins(&self) -> PinState {
        PinState::from_bits_truncate(self.pins_state.load(Ordering::Acquire))
    }

    pub fn drain_outbound(&self) -> Vec<Bytes> {
        let mut queue = self
            .outbound_queue
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        std::mem::take(&mut *queue)
    }

    /// Discards queued output (used when a callback fails part-way through).
    pub fn clear_outbound(&self) {
        let mut queue = self
            .outbound_queue
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        queue.clear();
    }

    pub fn drain_logs(&self) -> Vec<String> {
        let mut logs = self.log_messages.lock().unwrap_or_else(|e| e.into_inner());
        std::mem::take(&mut *logs)
    }

    /// Registers the `serial` table and its methods in the Lua global environment
    pub fn register(&self, lua: &Lua) -> Result<()> {
        let globals = lua.globals();
        let serial: Table = globals.get("serial")?;

        // 1. serial.send(data)
        let queue = self.outbound_queue.clone();
        let send_fn = lua.create_function(move |_lua, val: Value| {
            let bytes = match val {
                Value::String(s) => {
                    let slice = s.as_bytes();
                    if slice.len() > MAX_SERIAL_SEND_BYTES {
                        return Err(mlua::Error::MemoryError(format!(
                            "serial.send payload ({} bytes) exceeds max frame size ({MAX_SERIAL_SEND_BYTES} bytes)",
                            slice.len()
                        )));
                    }
                    Bytes::copy_from_slice(slice.as_ref())
                }
                Value::Table(t) => {
                    let len = t.raw_len();
                    if len > MAX_SERIAL_SEND_BYTES {
                        return Err(mlua::Error::MemoryError(format!(
                            "serial.send table length ({len} bytes) exceeds max frame size ({MAX_SERIAL_SEND_BYTES} bytes)"
                        )));
                    }
                    let mut buf = Vec::with_capacity(len);
                    for i in 1..=len {
                        let b: u8 = t.get(i)?;
                        buf.push(b);
                    }
                    Bytes::from(buf)
                }
                _ => {
                    return Err(mlua::Error::RuntimeError(
                        "serial.send() requires string or byte table".into(),
                    ));
                }
            };
            let mut q = queue.lock().unwrap_or_else(|e| e.into_inner());
            let queued_bytes: usize = q.iter().map(|b| b.len()).sum();
            if queued_bytes.saturating_add(bytes.len()) > MAX_OUTBOUND_QUEUE_BYTES {
                return Err(mlua::Error::MemoryError(format!(
                    "serial.send outbound queue limit ({MAX_OUTBOUND_QUEUE_BYTES} bytes) exceeded"
                )));
            }
            q.push(bytes);
            Ok(())
        })?;
        serial.set("send", send_fn)?;

        // 2. serial.set_pins(table)
        type PinSetter = fn(&mut PinState, bool);
        let pins_out = self.pins_state.clone();
        let set_pins_fn = lua.create_function(move |_lua, t: Table| {
            let mut state = PinState::from_bits_truncate(pins_out.load(Ordering::Acquire));
            // mlua's `bool` conversion applies Lua truthiness (any non-nil,
            // non-false value becomes true), so match on `Value` explicitly:
            // a bogus type is a script bug that must be reported, not silently
            // coerced.
            let fields: [(&str, PinSetter); 6] = [
                ("rts", |s, v| s.set(PinState::RTS, v)),
                ("cts", |s, v| s.set(PinState::CTS, v)),
                ("dtr", |s, v| s.set(PinState::DTR, v)),
                ("dsr", |s, v| s.set(PinState::DSR, v)),
                ("dcd", |s, v| s.set(PinState::DCD, v)),
                ("ri", |s, v| s.set(PinState::RI, v)),
            ];
            for (name, set) in fields {
                match t.get::<Value>(name)? {
                    Value::Nil => {}
                    Value::Boolean(value) => set(&mut state, value),
                    other => {
                        return Err(mlua::Error::RuntimeError(format!(
                            "serial.set_pins: field '{name}' must be a boolean, got {}",
                            other.type_name()
                        )));
                    }
                }
            }
            pins_out.store(state.bits(), Ordering::Release);
            Ok(())
        })?;
        serial.set("set_pins", set_pins_fn)?;

        // 3. serial.get_pins()
        let pins_in = self.pins_state.clone();
        let get_pins_fn = lua.create_function(move |lua, ()| {
            let state = PinState::from_bits_truncate(pins_in.load(Ordering::Acquire));
            let t = lua.create_table()?;
            t.set("rts", state.contains(PinState::RTS))?;
            t.set("cts", state.contains(PinState::CTS))?;
            t.set("dtr", state.contains(PinState::DTR))?;
            t.set("dsr", state.contains(PinState::DSR))?;
            t.set("dcd", state.contains(PinState::DCD))?;
            t.set("ri", state.contains(PinState::RI))?;
            Ok(t)
        })?;
        serial.set("get_pins", get_pins_fn)?;

        // 4. serial.log(message)
        let logs = self.log_messages.clone();
        let log_fn = lua.create_function(move |_lua, msg: String| {
            logs.lock().unwrap_or_else(|e| e.into_inner()).push(msg);
            Ok(())
        })?;
        serial.set("log", log_fn)?;

        Ok(())
    }
}

pub fn pin_state_to_lua(lua: &Lua, pins: PinState) -> Result<Table> {
    let t = lua.create_table()?;
    t.set("rts", pins.contains(PinState::RTS))?;
    t.set("cts", pins.contains(PinState::CTS))?;
    t.set("dtr", pins.contains(PinState::DTR))?;
    t.set("dsr", pins.contains(PinState::DSR))?;
    t.set("dcd", pins.contains(PinState::DCD))?;
    t.set("ri", pins.contains(PinState::RI))?;
    Ok(t)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sandbox::apply_sandbox;

    #[test]
    fn test_host_bridge_send_and_pins() {
        let lua = Lua::new();
        apply_sandbox(&lua).unwrap();

        let bridge = HostBridge::new();
        bridge.register(&lua).unwrap();

        // 1. Test string send
        lua.load("serial.send('HELLO_LUA')").exec().unwrap();
        let outbound = bridge.drain_outbound();
        assert_eq!(outbound.len(), 1);
        assert_eq!(&outbound[0][..], b"HELLO_LUA");

        // 2. Test table send
        lua.load("serial.send({ 0xAA, 0xBB, 0xCC })")
            .exec()
            .unwrap();
        let outbound2 = bridge.drain_outbound();
        assert_eq!(outbound2.len(), 1);
        assert_eq!(&outbound2[0][..], &[0xAA, 0xBB, 0xCC]);

        // 3. Test pin setting & getting
        lua.load("serial.set_pins({ rts = true, dtr = true, dcd = false })")
            .exec()
            .unwrap();
        let current_pins = bridge.get_pins();
        assert!(current_pins.contains(PinState::RTS));
        assert!(current_pins.contains(PinState::DTR));
        assert!(!current_pins.contains(PinState::DCD));

        let check: bool = lua.load("return serial.get_pins().rts").eval().unwrap();
        assert!(check);
    }
}
