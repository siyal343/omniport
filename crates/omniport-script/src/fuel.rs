use mlua::{HookTriggers, Lua, Result, VmState};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

pub const DEFAULT_MAX_FUEL: u64 = 100_000;
pub const DEFAULT_CHECK_INTERVAL: u32 = 100;

#[derive(Clone)]
pub struct FuelMeter {
    max_fuel: u64,
    check_interval: u32,
    consumed: Arc<AtomicU64>,
}

impl FuelMeter {
    pub fn new(max_fuel: u64, check_interval: u32) -> Self {
        Self {
            max_fuel,
            check_interval,
            consumed: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Resets consumed fuel before entering a new event callback
    pub fn reset(&self) {
        self.consumed.store(0, Ordering::Release);
    }

    /// Returns the number of instructions consumed in the current slice
    pub fn consumed_instructions(&self) -> u64 {
        self.consumed.load(Ordering::Acquire)
    }

    pub fn max_fuel(&self) -> u64 {
        self.max_fuel
    }

    /// Attaches the instruction counting hook to the Lua VM
    pub fn attach_hook(&self, lua: &Lua) -> Result<()> {
        let consumed = self.consumed.clone();
        let max_fuel = self.max_fuel;
        let interval = self.check_interval as u64;

        let triggers = HookTriggers::default().every_nth_instruction(self.check_interval);

        lua.set_hook(triggers, move |_lua, _debug| {
            let current = consumed.fetch_add(interval, Ordering::Relaxed) + interval;
            if current >= max_fuel {
                return Err(mlua::Error::RuntimeError(format!(
                    "FuelExceeded: consumed {} instructions (limit: {})",
                    current, max_fuel
                )));
            }
            Ok(VmState::Continue)
        });

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fuel_meter_terminates_runaway_loop() {
        let lua = Lua::new();
        let meter = FuelMeter::new(5_000, 50);
        meter.attach_hook(&lua).unwrap();

        let res: Result<()> = lua.load("local x = 0; while true do x = x + 1 end").exec();
        assert!(res.is_err(), "Runaway loop must be aborted");
        let msg = res.unwrap_err().to_string();
        assert!(msg.contains("FuelExceeded"));
        assert!(meter.consumed_instructions() >= 5_000);
    }

    #[test]
    fn test_normal_script_runs_within_fuel() {
        let lua = Lua::new();
        let meter = FuelMeter::new(10_000, 50);
        meter.attach_hook(&lua).unwrap();

        meter.reset();
        let sum: Result<i64> = lua
            .load("local s = 0; for i = 1, 100 do s = s + i end; return s")
            .eval();
        assert_eq!(sum.unwrap(), 5050);
        assert!(meter.consumed_instructions() < 10_000);
    }
}
