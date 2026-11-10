use thiserror::Error;

#[derive(Error, Debug)]
pub enum ScriptError {
    #[error("Lua error: {0}")]
    Lua(#[from] mlua::Error),

    #[error("Instruction fuel limit exceeded: consumed {consumed} instructions (limit: {limit})")]
    FuelExceeded { consumed: u64, limit: u64 },

    #[error("Script memory limit exceeded: {used} bytes allocated (limit: {limit} bytes)")]
    MemoryExceeded { used: usize, limit: usize },

    #[error("Script compilation error: {0}")]
    CompileError(String),

    #[error("Callback error in '{callback}': {reason}")]
    CallbackError { callback: String, reason: String },

    #[error("Sandboxing violation: attempt to access forbidden global '{0}'")]
    SandboxViolation(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, ScriptError>;
