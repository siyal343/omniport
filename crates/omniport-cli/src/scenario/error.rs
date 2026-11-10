use thiserror::Error;

#[derive(Error, Debug)]
pub enum ScenarioError {
    #[error("Scenario YAML parse error: {0}")]
    ParseError(String),

    #[error("Configuration error: {0}")]
    ConfigError(String),

    #[error("Assertion failed: {0}")]
    AssertionFailed(String),

    #[error("Step timed out: {0}")]
    Timeout(String),

    #[error("I/O error: {0}")]
    IoError(#[from] std::io::Error),

    #[error("Routing error: {0}")]
    RoutingError(#[from] omniport_core::routing::RoutingError),

    #[error("Topology error: {0}")]
    TopologyError(#[from] omniport_core::topology::TopologyError),

    #[error("Script error: {0}")]
    ScriptError(String),
}
