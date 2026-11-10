//! Static validation for declarative scenario files.
//!
//! A step or field that cannot take effect must fail at load time, not be
//! silently skipped: a green CI run must mean the YAML described what actually
//! happened. This module owns the "supported subset" rules for the scenario
//! runner; `docs/SCENARIO.md` documents the same contract for humans.

use super::error::ScenarioError;
use super::schema::{EndpointType, ScenarioConfig, StepConfig};
use std::collections::HashSet;

fn endpoint_type_label(kind: EndpointType) -> &'static str {
    match kind {
        EndpointType::Mock => "mock",
        EndpointType::Script => "script",
        EndpointType::TcpServer => "tcp_server",
        EndpointType::TcpClient => "tcp_client",
        EndpointType::Udp => "udp",
    }
}

fn step_action_name(step: &StepConfig) -> &'static str {
    match step {
        StepConfig::Sleep { .. } => "sleep",
        StepConfig::Send { .. } => "send",
        StepConfig::Expect { .. } => "expect",
        StepConfig::SetPins { .. } => "set_pins",
        StepConfig::AssertPins { .. } => "assert_pins",
        StepConfig::AssertMetrics { .. } => "assert_metrics",
    }
}

fn step_endpoint(step: &StepConfig) -> Option<u16> {
    match step {
        StepConfig::Send { endpoint, .. }
        | StepConfig::Expect { endpoint, .. }
        | StepConfig::SetPins { endpoint, .. }
        | StepConfig::AssertPins { endpoint, .. } => Some(*endpoint),
        StepConfig::Sleep { .. } | StepConfig::AssertMetrics { .. } => None,
    }
}

/// Validates everything the runner can know before it starts executing:
/// supported version, endpoint types/fields, route shape, and per-step
/// requirements.
pub(crate) fn validate_scenario(config: &ScenarioConfig) -> Result<(), ScenarioError> {
    if !config.version.starts_with("1.") {
        return Err(ScenarioError::ConfigError(format!(
            "unsupported scenario version '{}': this runner supports 1.x",
            config.version
        )));
    }
    if config.name.trim().is_empty() {
        return Err(ScenarioError::ConfigError(
            "scenario 'name' cannot be empty".to_string(),
        ));
    }

    // ---- Endpoints -------------------------------------------------------
    let mut endpoint_ids: HashSet<u16> = HashSet::new();
    for (index, ep) in config.endpoints.iter().enumerate() {
        let label = format!("endpoint #{} ('{}', id {})", index + 1, ep.name, ep.id);
        if ep.name.trim().is_empty() {
            return Err(ScenarioError::ConfigError(format!(
                "{label}: 'name' cannot be empty"
            )));
        }
        if !endpoint_ids.insert(ep.id) {
            return Err(ScenarioError::ConfigError(format!(
                "{label}: duplicate endpoint id {}",
                ep.id
            )));
        }

        match ep.endpoint_type {
            EndpointType::Mock | EndpointType::Script => {}
            EndpointType::TcpServer | EndpointType::TcpClient | EndpointType::Udp => {
                return Err(ScenarioError::ConfigError(format!(
                    "{label}: endpoint type '{}' is not supported by the scenario runner. \
                     Use a 'mock' or 'script' endpoint (network endpoints are owned by omniportd).",
                    endpoint_type_label(ep.endpoint_type)
                )));
            }
        }

        if ep.address.is_some() {
            return Err(ScenarioError::ConfigError(format!(
                "{label}: 'address' only applies to network endpoint types, which the scenario \
                 runner does not support"
            )));
        }

        match ep.endpoint_type {
            EndpointType::Script => {
                let has_script = ep
                    .script_path
                    .as_deref()
                    .map(|path| !path.trim().is_empty())
                    .unwrap_or(false);
                if !has_script {
                    return Err(ScenarioError::ConfigError(format!(
                        "{label}: script endpoints require a non-empty 'script_path'"
                    )));
                }
            }
            _ => {
                if ep.script_path.is_some() {
                    return Err(ScenarioError::ConfigError(format!(
                        "{label}: 'script_path' is only valid for script endpoints"
                    )));
                }
                if ep.fuel_limit.is_some() {
                    return Err(ScenarioError::ConfigError(format!(
                        "{label}: 'fuel_limit' is only valid for script endpoints"
                    )));
                }
            }
        }
    }

    // ---- Routes ----------------------------------------------------------
    for (index, route) in config.routes.iter().enumerate() {
        let label = format!("route #{} (source {})", index + 1, route.source);
        if route.id != 0 {
            return Err(ScenarioError::ConfigError(format!(
                "{label}: explicit 'id' is not supported — the daemon/core allocates canonical \
                 route ids. Remove the 'id' field."
            )));
        }
        if !endpoint_ids.contains(&route.source) {
            return Err(ScenarioError::ConfigError(format!(
                "{label}: source endpoint {} is not declared",
                route.source
            )));
        }
        if route.targets.is_empty() {
            return Err(ScenarioError::ConfigError(format!(
                "{label}: 'targets' cannot be empty"
            )));
        }
        let mut seen_targets = HashSet::new();
        for target in &route.targets {
            if *target == route.source {
                return Err(ScenarioError::ConfigError(format!(
                    "{label}: a route cannot target its own source endpoint"
                )));
            }
            if !endpoint_ids.contains(target) {
                return Err(ScenarioError::ConfigError(format!(
                    "{label}: target endpoint {target} is not declared"
                )));
            }
            if !seen_targets.insert(*target) {
                return Err(ScenarioError::ConfigError(format!(
                    "{label}: duplicate target endpoint {target}"
                )));
            }
        }
    }

    // ---- Capture ---------------------------------------------------------
    let empty_capture_output = config
        .capture
        .as_ref()
        .is_some_and(|capture| capture.enabled && capture.output_pcapng.trim().is_empty());
    if empty_capture_output {
        return Err(ScenarioError::ConfigError(
            "capture.output_pcapng cannot be empty when capture is enabled".to_string(),
        ));
    }

    // ---- Steps -----------------------------------------------------------
    for (index, step) in config.steps.iter().enumerate() {
        let label = format!("step #{} ({})", index + 1, step_action_name(step));
        if let Some(endpoint) = step_endpoint(step).filter(|ep| !endpoint_ids.contains(ep)) {
            return Err(ScenarioError::ConfigError(format!(
                "{label}: endpoint {endpoint} is not declared"
            )));
        }

        match step {
            StepConfig::Sleep { .. } => {}
            StepConfig::Send {
                payload_hex,
                payload_text,
                ..
            } => match (payload_hex, payload_text) {
                (Some(_), Some(_)) => {
                    return Err(ScenarioError::ConfigError(format!(
                        "{label}: set exactly one of 'payload_hex' or 'payload_text' (both were set)"
                    )));
                }
                (None, None) => {
                    return Err(ScenarioError::ConfigError(format!(
                        "{label}: one of 'payload_hex' or 'payload_text' is required"
                    )));
                }
                (Some(hex), None) if hex.trim().is_empty() => {
                    return Err(ScenarioError::ConfigError(format!(
                        "{label}: 'payload_hex' cannot be empty"
                    )));
                }
                (None, Some(text)) if text.is_empty() => {
                    return Err(ScenarioError::ConfigError(format!(
                        "{label}: 'payload_text' cannot be empty"
                    )));
                }
                _ => {}
            },
            StepConfig::Expect {
                match_hex,
                match_text,
                min_bytes,
                ..
            } => {
                if match_hex.is_none() && match_text.is_none() && min_bytes.is_none() {
                    return Err(ScenarioError::ConfigError(format!(
                        "{label}: requires at least one of 'match_hex', 'match_text' or 'min_bytes'"
                    )));
                }
            }
            StepConfig::SetPins { rts, dtr, brk, .. } => {
                if rts.is_none() && dtr.is_none() && brk.is_none() {
                    return Err(ScenarioError::ConfigError(format!(
                        "{label}: requires at least one of 'rts', 'dtr' or 'brk'"
                    )));
                }
            }
            StepConfig::AssertPins {
                rts,
                cts,
                dtr,
                dsr,
                dcd,
                ri,
                ..
            } => {
                if [rts, cts, dtr, dsr, dcd, ri]
                    .iter()
                    .all(|pin| pin.is_none())
                {
                    return Err(ScenarioError::ConfigError(format!(
                        "{label}: requires at least one expected pin (rts/cts/dtr/dsr/dcd/ri)"
                    )));
                }
            }
            StepConfig::AssertMetrics {
                min_packets,
                max_drops,
            } => {
                if min_packets.is_none() && max_drops.is_none() {
                    return Err(ScenarioError::ConfigError(format!(
                        "{label}: requires at least one of 'min_packets' or 'max_drops'"
                    )));
                }
            }
        }
    }

    Ok(())
}

/// Rejects unknown keys inside `steps` entries.
///
/// `StepConfig` is an internally tagged enum, where serde cannot apply
/// `deny_unknown_fields`; without this check a typo such as `timeout: 500`
/// would be silently ignored and the step would run with the default.
pub(crate) fn validate_raw_step_keys(yaml: &str) -> Result<(), String> {
    let value: serde_norway::Value =
        serde_norway::from_str(yaml).map_err(|e| format!("invalid YAML: {e}"))?;
    let Some(steps) = value.get("steps").and_then(|steps| steps.as_sequence()) else {
        return Ok(());
    };

    for (index, step) in steps.iter().enumerate() {
        let label = format!("step #{}", index + 1);
        let mapping = step
            .as_mapping()
            .ok_or_else(|| format!("{label}: every step must be a mapping"))?;
        let action_key = serde_norway::Value::String("action".to_string());
        let action = mapping
            .get(&action_key)
            .and_then(|action| action.as_str())
            .ok_or_else(|| format!("{label}: missing string 'action'"))?;

        let allowed: &[&str] = match action {
            "sleep" => &["action", "duration_ms"],
            "send" => &["action", "endpoint", "payload_hex", "payload_text"],
            "expect" => &[
                "action",
                "endpoint",
                "timeout_ms",
                "match_hex",
                "match_text",
                "min_bytes",
            ],
            "set_pins" => &["action", "endpoint", "rts", "dtr", "brk"],
            "assert_pins" => &[
                "action",
                "endpoint",
                "timeout_ms",
                "rts",
                "cts",
                "dtr",
                "dsr",
                "dcd",
                "ri",
            ],
            "assert_metrics" => &["action", "min_packets", "max_drops"],
            other => {
                return Err(format!(
                    "{label}: unknown action '{other}' (expected sleep/send/expect/set_pins/assert_pins/assert_metrics)"
                ));
            }
        };

        for key in mapping.keys() {
            let key = key
                .as_str()
                .ok_or_else(|| format!("{label}: field names must be strings"))?;
            if !allowed.contains(&key) {
                return Err(format!(
                    "{label} ({action}): unknown field '{key}' — supported fields: {}",
                    allowed.join(", ")
                ));
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scenario::schema::{
        EndpointConfig, EndpointType, RouteConfig, ScenarioConfig, StepConfig,
    };

    fn base() -> ScenarioConfig {
        ScenarioConfig {
            version: "1.0".to_string(),
            name: "test".to_string(),
            description: None,
            endpoints: vec![EndpointConfig {
                id: 1,
                name: "ep1".to_string(),
                endpoint_type: EndpointType::Mock,
                baud_rate: 115_200,
                script_path: None,
                fuel_limit: None,
                address: None,
            }],
            routes: vec![],
            capture: None,
            steps: vec![],
        }
    }

    #[test]
    fn test_valid_minimal_scenario_passes() {
        validate_scenario(&base()).expect("minimal scenario should validate");
    }

    #[test]
    fn test_network_endpoint_types_are_rejected() {
        for endpoint_type in [
            EndpointType::TcpServer,
            EndpointType::TcpClient,
            EndpointType::Udp,
        ] {
            let mut config = base();
            config.endpoints[0].endpoint_type = endpoint_type;
            config.endpoints[0].address = Some("127.0.0.1:9000".to_string());
            let err = validate_scenario(&config).unwrap_err().to_string();
            assert!(
                err.contains("not supported"),
                "unexpected error for {endpoint_type:?}: {err}"
            );
        }
    }

    #[test]
    fn test_route_id_and_unknown_references_are_rejected() {
        let mut config = base();
        config.routes.push(RouteConfig {
            id: 7,
            source: 1,
            targets: vec![2],
            bidirectional: false,
            fault: None,
        });
        assert!(
            validate_scenario(&config)
                .unwrap_err()
                .to_string()
                .contains("explicit 'id'")
        );

        config.routes[0].id = 0;
        assert!(
            validate_scenario(&config)
                .unwrap_err()
                .to_string()
                .contains("not declared")
        );
    }

    #[test]
    fn test_step_requirements_are_enforced() {
        let mut config = base();
        config.steps.push(StepConfig::Send {
            endpoint: 1,
            payload_hex: Some("00".into()),
            payload_text: Some("x".into()),
        });
        assert!(validate_scenario(&config).is_err());

        let mut config = base();
        config.steps.push(StepConfig::SetPins {
            endpoint: 1,
            rts: None,
            dtr: None,
            brk: None,
        });
        assert!(validate_scenario(&config).is_err());

        let mut config = base();
        config.steps.push(StepConfig::AssertPins {
            endpoint: 1,
            timeout_ms: 100,
            rts: None,
            cts: None,
            dtr: None,
            dsr: None,
            dcd: None,
            ri: None,
        });
        assert!(validate_scenario(&config).is_err());

        let mut config = base();
        config.steps.push(StepConfig::AssertMetrics {
            min_packets: None,
            max_drops: None,
        });
        assert!(validate_scenario(&config).is_err());
    }
}
