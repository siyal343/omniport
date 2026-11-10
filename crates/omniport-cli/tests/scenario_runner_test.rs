use omniport_cli::scenario::ScenarioRunner;
use std::io::Write;
use tempfile::NamedTempFile;

#[tokio::test]
async fn test_basic_mock_echo_scenario() {
    let yaml = r#"
version: "1.0"
name: "Mock Echo CI Test"

endpoints:
  - id: 1
    name: "sender"
    type: "mock"
    baud_rate: 115200

  - id: 2
    name: "receiver"
    type: "mock"
    baud_rate: 115200

routes:
  - source: 1
    targets: [2]
    bidirectional: false

steps:
  - action: "sleep"
    duration_ms: 10

  - action: "send"
    endpoint: 1
    payload_hex: "010300000002C40B"

  - action: "expect"
    endpoint: 2
    timeout_ms: 500
    match_hex: "010300000002C40B"

  - action: "send"
    endpoint: 1
    payload_text: "HELLO_OMNIPORT"

  - action: "expect"
    endpoint: 2
    timeout_ms: 500
    match_text: "HELLO_OMNIPORT"

  - action: "set_pins"
    endpoint: 1
    rts: true
    dtr: true

  - action: "assert_pins"
    endpoint: 1
    rts: true
    dtr: true

  - action: "assert_metrics"
    min_packets: 2
"#;

    let mut runner = ScenarioRunner::from_yaml(yaml).expect("Failed to parse scenario");
    let report = runner.run().await;

    assert!(report.success, "Scenario should pass: {:?}", report.error);
    assert_eq!(report.total_steps, 8);
    assert_eq!(report.passed_steps, 8);
    assert_eq!(report.failed_steps, 0);
}

#[tokio::test]
async fn test_script_simulator_scenario() {
    let mut script_file = NamedTempFile::new().unwrap();
    let lua_code = r#"
        function on_rx(payload, pins)
            serial.send("PONG:" .. payload)
        end
    "#;
    script_file.write_all(lua_code.as_bytes()).unwrap();
    let script_path = script_file.path().to_str().unwrap().replace('\\', "/");

    let yaml = format!(
        r#"
version: "1.0"
name: "Lua Simulator Scenario"

endpoints:
  - id: 1
    name: "tester"
    type: "mock"
    baud_rate: 115200

  - id: 2
    name: "lua_device"
    type: "script"
    script_path: "{script_path}"
    fuel_limit: 100000
    baud_rate: 115200

routes:
  - source: 1
    targets: [2]
    bidirectional: true

steps:
  - action: "send"
    endpoint: 1
    payload_text: "PING_REQUEST"

  - action: "expect"
    endpoint: 1
    timeout_ms: 1000
    match_text: "PONG:PING_REQUEST"
"#
    );

    let mut runner = ScenarioRunner::from_yaml(&yaml).expect("Failed to parse scenario");
    let report = runner.run().await;

    assert!(report.success, "Scenario should pass: {:?}", report.error);
    assert_eq!(report.passed_steps, 2);
}

#[tokio::test]
async fn test_fault_injection_jitter_scenario() {
    let yaml = r#"
version: "1.0"
name: "Fault Injection Latency Scenario"

endpoints:
  - id: 1
    name: "sender"
    type: "mock"

  - id: 2
    name: "receiver"
    type: "mock"

routes:
  - source: 1
    targets: [2]
    bidirectional: false
    fault:
      mean_latency_ms: 15.0
      jitter_stddev_ms: 2.0

steps:
  - action: "send"
    endpoint: 1
    payload_text: "DELAYED_PACKET"

  - action: "expect"
    endpoint: 2
    timeout_ms: 500
    match_text: "DELAYED_PACKET"
"#;

    let mut runner = ScenarioRunner::from_yaml(yaml).expect("Failed to parse scenario");
    let report = runner.run().await;

    assert!(
        report.success,
        "Delayed packet should arrive and pass: {:?}",
        report.error
    );
    assert_eq!(report.passed_steps, 2);
}

#[tokio::test]
async fn test_pcapng_capture_scenario() {
    let temp_pcap = tempfile::Builder::new()
        .suffix(".pcapng")
        .tempfile()
        .unwrap();
    let pcap_path = temp_pcap.path().to_str().unwrap().replace('\\', "/");

    let yaml = format!(
        r#"
version: "1.0"
name: "PCAPNG Capture Test"

endpoints:
  - id: 1
    name: "sender"
    type: "mock"

  - id: 2
    name: "receiver"
    type: "mock"

routes:
  - source: 1
    targets: [2]

capture:
  enabled: true
  output_pcapng: "{pcap_path}"

steps:
  - action: "send"
    endpoint: 1
    payload_hex: "DEADBEEFCAFE"

  - action: "expect"
    endpoint: 2
    timeout_ms: 500
    match_hex: "DEADBEEFCAFE"
"#
    );

    let mut runner = ScenarioRunner::from_yaml(&yaml).expect("Failed to parse scenario");
    let report = runner.run().await;

    assert!(report.success, "Capture test should succeed");
    drop(runner);

    // Verify PCAPNG file exists and contains binary data
    let metadata = std::fs::metadata(temp_pcap.path()).expect("PCAPNG file should exist");
    assert!(
        metadata.len() > 64,
        "PCAPNG file must contain headers and packets"
    );
}

#[tokio::test]
async fn test_drop_rate_full_disconnect_actually_drops_packets() {
    // drop_rate: 1.0 is the documented "packet drop rate" extreme and must map
    // onto the fault injector's full-disconnect mode. Previously the field was
    // parsed then silently ignored, so the packet still arrived.
    let yaml = r#"
version: "1.0"
name: "Full Drop Scenario"

endpoints:
  - id: 1
    name: "sender"
    type: "mock"
  - id: 2
    name: "receiver"
    type: "mock"

routes:
  - source: 1
    targets: [2]
    fault:
      drop_rate: 1.0

steps:
  - action: "send"
    endpoint: 1
    payload_text: "MUST_BE_DROPPED"
  - action: "expect"
    endpoint: 2
    timeout_ms: 100
    match_text: "MUST_BE_DROPPED"
"#;

    let mut runner = ScenarioRunner::from_yaml(yaml).expect("scenario should parse");
    let report = runner.run().await;

    assert!(
        !report.success,
        "drop_rate: 1.0 must drop the frame, but the expect step matched it"
    );
    assert!(
        report.error.unwrap().contains("Timed out"),
        "expected an expectation timeout caused by the drop"
    );
}

#[tokio::test]
async fn test_fault_drop_rate_zero_keeps_packets_flowing() {
    let yaml = r#"
version: "1.0"
name: "No Drop Scenario"

endpoints:
  - id: 1
    name: "sender"
    type: "mock"
  - id: 2
    name: "receiver"
    type: "mock"

routes:
  - source: 1
    targets: [2]
    fault:
      drop_rate: 0.0

steps:
  - action: "send"
    endpoint: 1
    payload_text: "DELIVERED"
  - action: "expect"
    endpoint: 2
    timeout_ms: 500
    match_text: "DELIVERED"
"#;

    let mut runner = ScenarioRunner::from_yaml(yaml).expect("scenario should parse");
    let report = runner.run().await;
    assert!(report.success, "expected pass: {:?}", report.error);
}

#[tokio::test]
async fn test_unsupported_fractional_drop_rate_is_rejected_loudly() {
    // The fault injector has no per-frame Bernoulli drop; a fractional rate must
    // be an explicit configuration error, never a silent no-op.
    let yaml = r#"
version: "1.0"
name: "Fractional Drop Scenario"

endpoints:
  - id: 1
    name: "sender"
    type: "mock"
  - id: 2
    name: "receiver"
    type: "mock"

routes:
  - source: 1
    targets: [2]
    fault:
      drop_rate: 0.5
"#;

    let err = match ScenarioRunner::from_yaml(yaml) {
        Ok(_) => panic!("fractional drop_rate must be rejected"),
        Err(e) => e,
    };
    let msg = err.to_string();
    assert!(
        msg.contains("drop_rate") && msg.contains("not supported"),
        "unexpected error: {msg}"
    );
}

#[tokio::test]
async fn test_out_of_range_fault_probabilities_are_rejected() {
    for field in ["drop_rate", "bit_error_rate"] {
        let yaml = format!(
            r#"
version: "1.0"
name: "Bad Fault Scenario"
endpoints:
  - id: 1
    name: "sender"
    type: "mock"
  - id: 2
    name: "receiver"
    type: "mock"
routes:
  - source: 1
    targets: [2]
    fault:
      {field}: 1.5
"#
        );
        let err = match ScenarioRunner::from_yaml(&yaml) {
            Ok(_) => panic!("{field}: 1.5 must be rejected"),
            Err(e) => e,
        };
        assert!(
            err.to_string().contains(field),
            "error should name the bad field {field}: {err}"
        );
    }
}

#[tokio::test]
async fn test_steps_referencing_undeclared_endpoints_fail_loudly() {
    // A typo in the step endpoint used to be a silent no-op: `send` routed to
    // zero destinations and the scenario reported PASS. It is now rejected at
    // load time, before any step runs.
    let yaml = r#"
version: "1.0"
name: "Undeclared Endpoint Scenario"

endpoints:
  - id: 1
    name: "sender"
    type: "mock"

steps:
  - action: "send"
    endpoint: 99
    payload_text: "LOST"
"#;

    let err = match ScenarioRunner::from_yaml(yaml) {
        Ok(_) => panic!("undeclared step endpoint must be rejected at load time"),
        Err(err) => err,
    };
    assert!(
        err.to_string().contains("endpoint 99 is not declared"),
        "error should identify the undeclared endpoint: {err}"
    );
}

#[tokio::test]
async fn test_expectation_timeout_failure() {
    let yaml = r#"
version: "1.0"
name: "Timeout Failure Scenario"

endpoints:
  - id: 1
    name: "sender"
    type: "mock"

  - id: 2
    name: "receiver"
    type: "mock"

routes:
  - source: 1
    targets: [2]

steps:
  # Expect data that is never sent
  - action: "expect"
    endpoint: 2
    timeout_ms: 50
    match_text: "NEVER_ARRIVES"
"#;

    let mut runner = ScenarioRunner::from_yaml(yaml).expect("Failed to parse scenario");
    let report = runner.run().await;

    assert!(!report.success, "Scenario should fail due to timeout");
    assert_eq!(report.failed_steps, 1);
    assert!(report.error.unwrap().contains("Timed out"));
}

#[test]
fn test_scenario_rejects_route_referencing_undeclared_endpoint_or_self_loop() {
    let missing_target_yaml = r#"
version: "1.0"
name: "Missing Route Endpoint"
endpoints:
  - id: 1
    name: "sender"
    type: "mock"
routes:
  - source: 1
    targets: [99]
"#;
    let err = match ScenarioRunner::from_yaml(missing_target_yaml) {
        Ok(_) => panic!("route targeting missing endpoint must fail"),
        Err(e) => e,
    };
    assert!(
        err.to_string().contains("99"),
        "error should name missing endpoint: {err}"
    );

    let self_loop_yaml = r#"
version: "1.0"
name: "Self Loop Route"
endpoints:
  - id: 1
    name: "sender"
    type: "mock"
routes:
  - source: 1
    targets: [1]
"#;
    let err = match ScenarioRunner::from_yaml(self_loop_yaml) {
        Ok(_) => panic!("self-loop route must fail"),
        Err(e) => e,
    };
    assert!(
        err.to_string().contains("1"),
        "error should explain self-loop: {err}"
    );
}

#[tokio::test]
async fn test_scenario_fanout_bidirectional_route_via_topology() {
    let yaml = r#"
version: "1.0"
name: "Fanout Bidi"
endpoints:
  - id: 1
    name: "hub"
    type: "mock"
  - id: 2
    name: "leaf_a"
    type: "mock"
  - id: 3
    name: "leaf_b"
    type: "mock"
routes:
  - source: 1
    targets: [2, 3]
    bidirectional: true
steps:
  - action: "send"
    endpoint: 1
    payload_text: "BROADCAST"
  - action: "expect"
    endpoint: 2
    timeout_ms: 200
    match_text: "BROADCAST"
  - action: "expect"
    endpoint: 3
    timeout_ms: 200
    match_text: "BROADCAST"
  - action: "send"
    endpoint: 3
    payload_text: "REPLY_FROM_B"
  - action: "expect"
    endpoint: 1
    timeout_ms: 200
    match_text: "REPLY_FROM_B"
"#;
    let mut runner = ScenarioRunner::from_yaml(yaml).expect("scenario should parse");
    let report = runner.run().await;
    assert!(report.success, "scenario should pass: {:?}", report.error);
    assert_eq!(report.passed_steps, 5);
}
