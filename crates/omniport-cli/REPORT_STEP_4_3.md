# OmniPort Step 4.3 Verification Report: Declarative CI Scenario Runner in `omniport-cli`

**Date:** 2026-11-10  
**Phase:** 4 — Simulation, RFC 2217 Networking & CI Test Harness  
**Step:** 4.3 — Build declarative `.omniport.yaml` CI scenario runner in `omniport-cli`  
**Status:** PASSED (Verification Complete)

---

## 1. Architectural Context & Requirements

OmniPort serves as an industrial-grade Hardware-In-the-Loop (HIL) fault-injection and virtual serial testing platform. Continuous Integration (CI) and automated regression pipelines require a completely headless, declarative mechanism to:
- Spin up multi-port serial topologies without manual UI interaction or daemon pre-configuration.
- Connect virtual mock endpoints, sandboxed Lua 5.4 device simulators (`omniport-script`), and network serial servers (`omniport-net`).
- Dynamically apply physical-layer and transport-layer fault injection profiles (Gaussian jitter, packet drops, BER, burst errors).
- Execute timed sequences of serial frame transmissions (hex, ASCII) and assertions (payload pattern matching, modem control pin states, metrics).
- Record and export binary traffic directly to PCAPNG format for post-mortem analysis in Wireshark.
- Report structured results to console stdout or machine-readable JSON with standardized exit codes (`0` for pass, `1` for fail).

---

## 2. Component Implementation Details

### 2.1 Declarative Schema (`crates/omniport-cli/src/scenario/schema.rs`)
The scenario runner parses `.omniport.yaml` configurations specifying:
- **`endpoints`**:
  - `id`: Unique endpoint numeric identifier ($1 \dots 65535$).
  - `name`: Human-readable identifier (e.g. `hil_master`, `dut_slave`).
  - `type`: `mock`, `script`, `tcp_server`, `tcp_client`, `udp`.
  - `baud_rate`: Virtual baud rate ($115200$, $921600$, etc.).
  - `script_path` & `fuel_limit`: Embedded Lua script parameters for `omniport-script`.
- **`routes`**:
  - `source` & `targets`: Directed or bidirectional routing topology.
  - `fault`: Configurable Gaussian jitter (`mean_latency_ms`, `jitter_stddev_ms`), packet drop rate (`drop_rate`), bit error rate (`bit_error_rate`), and CRC corruption (`corrupt_nth_crc`).
- **`capture`**:
  - `enabled`: Boolean toggle.
  - `output_pcapng`: Output file path for Wireshark PCAPNG capture.
- **`steps`**:
  - `sleep`: Sleep for `duration_ms`.
  - `send`: Transmit raw binary payload via `payload_hex` or ASCII string via `payload_text`.
  - `expect`: Await event matching `match_hex`, `match_text`, or `min_bytes` within `timeout_ms`.
  - `set_pins`: Assert or de-assert hardware modem control pins (`rts`, `dtr`, `brk`).
  - `assert_pins`: Assert expected pin states (`rts`, `cts`, `dtr`, `dsr`, `dcd`, `ri`).
  - `assert_metrics`: Assert packet counters (`min_packets`, `max_drops`).

### 2.2 Unified `AsyncEndpoint` Bridge & Execution Engine (`crates/omniport-cli/src/scenario/runner.rs`)
- **Lock-Free Routing Graph:** Directly integrates `omniport_core::routing::RoutingGraph`.
- **Asynchronous Bridge Tasks:** Uses generic `spawn_endpoint_bridge<E: AsyncEndpoint>` with `tokio::select!` and cancellation channels to bridge external endpoints (Lua simulators, network endpoints) with the central routing graph.
- **Hex & Pattern Engine (`crates/omniport-cli/src/hex_utils.rs`):** Standard library zero-dependency hex encoder/decoder and byte window matcher.
- **Dual Mode Execution:**
  - Standalone mode (`omniport run <file>`): Runs completely in-process without requiring background daemon processes.
  - Machine-readable output (`--json`): Emits structured JSON summary containing per-step elapsed timings and assertion outcomes.

---

## 3. Verification & Benchmarks

### 3.1 Live CLI Execution Benchmark
Execution of root `.omniport.yaml` (10 steps including Modbus RTU query/response, ASCII commands, pin assertions, and PCAPNG capture):
```
============================================================
 OmniPort CI Scenario: OmniPort HIL Loopback & Fault Verification
============================================================
 [PASS]  1. sleep        Sleep 10ms                       ( 20.36ms)
 [PASS]  2. send         Send 8 bytes on ep 1             (  0.58ms)
 [PASS]  3. expect       Expect on ep 2 (timeout: 500ms)  ( 16.79ms)
 [PASS]  4. send         Send 9 bytes on ep 2             (  0.41ms)
 [PASS]  5. expect       Expect on ep 1 (timeout: 500ms)  ( 16.53ms)
 [PASS]  6. send         Send 12 bytes on ep 1            (  0.46ms)
 [PASS]  7. expect       Expect on ep 2 (timeout: 500ms)  ( 16.63ms)
 [PASS]  8. set_pins     Set pins on ep 1 (RTS=Some(true), DTR=Some(true)) (  0.12ms)
 [PASS]  9. assert_pins  Assert pins on ep 1              (  0.04ms)
 [PASS] 10. assert_metrics Assert scenario metrics          (  0.00ms)
------------------------------------------------------------
Result: PASSED (10/10 steps passed in 71.94ms)
PCAPNG Capture: target/ci_scenario.pcapng (364 bytes)
============================================================
```

### 3.2 Automated Test Suite (`cargo test -p omniport-cli`)
- `hex_utils::tests::test_hex_matches`: Verified hex pattern matching.
- `hex_utils::tests::test_hex_roundtrip`: Verified byte-to-hex roundtrip.
- `test_basic_mock_echo_scenario`: Verified full 8-step declarative mock scenario with Modbus hex, text, and pin assertions.
- `test_script_simulator_scenario`: Verified bidirectional loopback between mock endpoint and sandboxed Lua 5.4 device simulator (`on_rx` handler replying with `PONG:<payload>`).
- `test_fault_injection_jitter_scenario`: Verified frame delivery across a jittered/delayed route ($15\,\text{ms} \pm 2\,\text{ms}$).
- `test_pcapng_capture_scenario`: Verified PCAPNG capture file creation and header serialization.
- `test_expectation_timeout_failure`: Verified clean failure reporting and non-zero exit state on expectation timeout.

---

## 4. Architectural Invariants Sign-off
- [x] **Declarative CI Runner:** Fully implemented in `omniport-cli` with YAML parsing and step execution.
- [x] **Zero Background Daemon Required:** Runs standalone in CI runners with zero pre-requisite processes.
- [x] **Full Subsystem Integration:** Exercises `omniport-core`, `omniport-capture`, `omniport-script`, and `omniport-proto`.
- [x] **Strict Quality Standards:** Zero clippy warnings, zero unhandled errors, full clean teardown of async tasks.
