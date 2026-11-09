use bytes::Bytes;
use mlua::{Lua, Result, Table, Value};
use omniport_core::PinState;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex};

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
        let mut queue = self.outbound_queue.lock().unwrap();
        std::mem::take(&mut *queue)
    }

    pub fn drain_logs(&self) -> Vec<String> {
        let mut logs = self.log_messages.lock().unwrap();
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
                Value::String(s) => Bytes::copy_from_slice(s.as_bytes().as_ref()),
                Value::Table(t) => {
                    let len = t.raw_len();
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
                    ))
                }
            };
            queue.lock().unwrap().push(bytes);
            Ok(())
        })?;
        serial.set("send", send_fn)?;

        // 2. serial.set_pins(table)
        let pins_out = self.pins_state.clone();
        let set_pins_fn = lua.create_function(move |_lua, t: Table| {
            let mut state = PinState::from_bits_truncate(pins_out.load(Ordering::Acquire));
            if let Ok(v) = t.get::<bool>("rts") {
                state.set(PinState::RTS, v);
            }
            if let Ok(v) = t.get::<bool>("cts") {
                state.set(PinState::CTS, v);
            }
            if let Ok(v) = t.get::<bool>("dtr") {
                state.set(PinState::DTR, v);
            }
            if let Ok(v) = t.get::<bool>("dsr") {
                state.set(PinState::DSR, v);
            }
            if let Ok(v) = t.get::<bool>("dcd") {
                state.set(PinState::DCD, v);
            }
            if let Ok(v) = t.get::<bool>("ri") {
                state.set(PinState::RI, v);
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
            logs.lock().unwrap().push(msg);
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
        lua.load("serial.send({ 0xAA, 0xBB, 0xCC })").exec().unwrap();
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
