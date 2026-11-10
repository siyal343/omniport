//! Embedded Lua 5.4 (`mlua`) sandboxed device simulator for OmniPort.
//!
//! Provides fuel-limited Lua scripting, strict sandboxing (stripping IO/OS globals),
//! modem pin control hooks, and full `AsyncEndpoint` routing matrix integration.

pub mod bindings;
pub mod endpoint;
pub mod engine;
pub mod error;
pub mod fuel;
pub mod sandbox;

pub use bindings::MAX_SERIAL_SEND_BYTES;
pub use endpoint::ScriptEndpoint;
pub use engine::{DEFAULT_MAX_MEMORY_BYTES, ScriptConfig, ScriptEngine};
pub use error::{Result, ScriptError};
pub use fuel::{DEFAULT_CHECK_INTERVAL, DEFAULT_MAX_FUEL, FuelMeter};
