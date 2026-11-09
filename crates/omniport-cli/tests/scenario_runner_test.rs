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
  - id: 1
    source: 1
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
  - id: 1
    source: 1
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
  - id: 1
    source: 1
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

    assert!(report.success, "Delayed packet should arrive and pass: {:?}", report.error);
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
  - id: 1
    source: 1
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
    assert!(metadata.len() > 64, "PCAPNG file must contain headers and packets");
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
  - id: 1
    source: 1
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
