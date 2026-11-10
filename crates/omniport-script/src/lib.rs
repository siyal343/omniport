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

pub use endpoint::ScriptEndpoint;
pub use engine::{ScriptConfig, ScriptEngine};
pub use error::{Result, ScriptError};
pub use fuel::{DEFAULT_CHECK_INTERVAL, DEFAULT_MAX_FUEL, FuelMeter};
