//! Step 7.3 semantics tests: unsupported YAML is rejected at load time and
//! `assert_pins` honours its timeout / observes asynchronously applied pins.

use omniport_cli::scenario::ScenarioRunner;
use std::io::Write;
use tempfile::NamedTempFile;

fn load_error(yaml: &str) -> String {
    match ScenarioRunner::from_yaml(yaml) {
        Ok(_) => panic!("expected scenario load to fail"),
        Err(err) => err.to_string(),
    }
}

fn scenario(endpoint_type: &str, extra_endpoint: &str, steps: &str) -> String {
    format!(
        r#"
version: "1.0"
name: "semantics test"

endpoints:
  - id: 1
    name: "sender"
    type: "mock"
    baud_rate: 115200
  - id: 2
    name: "peer"
    type: "{endpoint_type}"
    baud_rate: 115200
{extra_endpoint}

routes:
  - source: 1
    targets: [2]
    bidirectional: true

steps:
{steps}
"#
    )
}

#[test]
fn test_network_endpoint_types_are_rejected_at_load_time() {
    for endpoint_type in ["tcp_server", "tcp_client", "udp"] {
        let yaml = scenario(
            endpoint_type,
            "    address: \"127.0.0.1:9000\"\n",
            "  - action: \"sleep\"\n    duration_ms: 1\n",
        );
        let err = match ScenarioRunner::from_yaml(&yaml) {
            Ok(_) => panic!("{endpoint_type} endpoints must be rejected"),
            Err(err) => err,
        };
        assert!(
            err.to_string().contains("not supported"),
            "{endpoint_type}: unexpected error: {err}"
        );
    }
}

#[test]
fn test_address_on_mock_endpoint_is_rejected() {
    let yaml = scenario(
        "mock",
        "    address: \"127.0.0.1:9000\"\n",
        "  - action: \"sleep\"\n    duration_ms: 1\n",
    );
    let err = load_error(&yaml);
    assert!(err.contains("'address'"), "unexpected error: {err}");
}

#[test]
fn test_explicit_route_id_is_rejected() {
    let yaml = r#"
version: "1.0"
name: "route id"
endpoints:
  - id: 1
    name: "a"
    type: "mock"
  - id: 2
    name: "b"
    type: "mock"
routes:
  - id: 7
    source: 1
    targets: [2]
steps:
  - action: "sleep"
    duration_ms: 1
"#;
    let err = load_error(yaml);
    assert!(err.contains("explicit 'id'"), "unexpected error: {err}");
}

#[test]
fn test_unknown_fields_are_rejected() {
    let yaml = r#"
version: "1.0"
name: "unknown field"
endpoints:
  - id: 1
    name: "a"
    type: "mock"
    bogus_field: 1
steps: []
"#;
    let err = load_error(yaml);
    assert!(
        err.contains("unknown field") || err.contains("bogus_field"),
        "unexpected error: {err}"
    );
}

#[test]
fn test_unsupported_version_is_rejected() {
    let yaml = r#"
version: "2.0"
name: "version"
steps: []
"#;
    let err = load_error(yaml);
    assert!(err.contains("version"), "unexpected error: {err}");
}

#[test]
fn test_step_fields_that_cannot_take_effect_are_rejected() {
    // Both payload forms.
    let yaml = scenario(
        "mock",
        "",
        r#"  - action: "send"
    endpoint: 1
    payload_hex: "00"
    payload_text: "x"
"#,
    );
    assert!(ScenarioRunner::from_yaml(&yaml).is_err());

    // Neither payload form.
    let yaml = scenario(
        "mock",
        "",
        r#"  - action: "send"
    endpoint: 1
"#,
    );
    assert!(ScenarioRunner::from_yaml(&yaml).is_err());

    // Vacuously passing pin/metric assertions.
    for steps in [
        r#"  - action: "set_pins"
    endpoint: 1
"#,
        r#"  - action: "assert_pins"
    endpoint: 2
"#,
        r#"  - action: "assert_metrics"
"#,
        r#"  - action: "expect"
    endpoint: 2
"#,
    ] {
        let yaml = scenario("mock", "", steps);
        assert!(
            ScenarioRunner::from_yaml(&yaml).is_err(),
            "step should be rejected: {steps}"
        );
    }
}

#[test]
fn test_unknown_step_fields_are_rejected() {
    let yaml = r#"
version: "1.0"
name: "unknown step field"
endpoints:
  - id: 1
    name: "a"
    type: "mock"
steps:
  - action: "expect"
    endpoint: 1
    timeout: 500
    match_hex: "00"
"#;
    let err = load_error(yaml);
    assert!(
        err.contains("unknown field 'timeout'"),
        "unexpected error: {err}"
    );
}

#[test]
fn test_unknown_step_action_is_rejected() {
    let yaml = r#"
version: "1.0"
name: "unknown action"
steps:
  - action: "explode"
"#;
    let err = load_error(yaml);
    assert!(err.contains("unknown action"), "unexpected error: {err}");
}

#[tokio::test]
async fn test_assert_pins_honours_timeout_before_failing() {
    let yaml = r#"
version: "1.0"
name: "assert pins timeout"
endpoints:
  - id: 1
    name: "peer"
    type: "mock"
steps:
  - action: "assert_pins"
    endpoint: 1
    timeout_ms: 120
    rts: true
"#;
    let mut runner = ScenarioRunner::from_yaml(yaml).unwrap();
    let report = runner.run().await;
    assert!(
        !report.success,
        "assert_pins must fail when the pin never changes"
    );
    let step = &report.steps[0];
    assert!(
        step.duration_ms >= 100.0,
        "assert_pins must wait for its timeout, only waited {}ms",
        step.duration_ms
    );
    let error = step.error.as_deref().unwrap_or_default();
    assert!(
        error.contains("RTS") && error.contains("after waiting 120ms"),
        "error should report the timed-out pin: {error}"
    );
}

#[tokio::test]
async fn test_assert_pins_observes_pin_change_applied_by_script_endpoint() {
    let mut script = NamedTempFile::new().expect("temp script");
    writeln!(
        script,
        r#"
function on_rx(payload, pins)
    serial.set_pins({{ cts = true }})
    serial.send("ACK")
end
"#
    )
    .expect("write script");

    let yaml = format!(
        r#"
version: "1.0"
name: "async pin observation"
endpoints:
  - id: 1
    name: "sender"
    type: "mock"
  - id: 2
    name: "scripted"
    type: "script"
    script_path: "{}"
routes:
  - source: 1
    targets: [2]
    bidirectional: true
steps:
  - action: "send"
    endpoint: 1
    payload_text: "PING"
  - action: "assert_pins"
    endpoint: 2
    timeout_ms: 2000
    cts: true
"#,
        script.path().display().to_string().replace('\\', "/")
    );

    let mut runner = ScenarioRunner::from_yaml(&yaml).expect("scenario should parse");
    let report = runner.run().await;
    assert!(
        report.success,
        "assert_pins should observe the script-applied CTS pin: {:?}",
        report.error
    );
    assert_eq!(report.passed_steps, 2);
}
