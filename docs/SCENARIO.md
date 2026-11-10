# Declarative CI Scenarios (`omniport run`)

`omniport-cli` executes a YAML scenario as a self-contained, deterministic
router test: mock/script endpoints, canonical routes, fault profiles, PCAPNG
capture, and asserted steps. It is the CI contract for the routing/faulting
pipeline — a green run must mean the YAML described what actually happened, so
**every field either takes effect or fails at load time**. The implementation
lives in `crates/omniport-cli/src/scenario/` (`schema.rs`, `validate.rs`,
`runner.rs`); the root `.omniport.yaml` is the reference scenario.

## File shape

```yaml
version: "1.0"            # required; the runner supports 1.x only
name: "UART echo CI"      # required, non-empty
description: "..."        # optional metadata

endpoints:                # at least the ids referenced below
  - id: 1
    name: "sender"
    type: "mock"          # mock | script (network types are rejected, see below)
    baud_rate: 115200
  - id: 2
    name: "device"
    type: "script"
    script_path: "scripts/device.lua"
    fuel_limit: 1000000   # optional, script endpoints only

routes:
  - source: 1
    targets: [2]
    bidirectional: true
    fault:                # optional
      drop_rate: 0.0
      bit_error_rate: 0.0
      mean_latency_ms: 5.0
      jitter_stddev_ms: 1.0

capture:                  # optional
  enabled: true
  output_pcapng: "artifacts/ci.pcapng"

steps:
  - action: "send"
    endpoint: 1
    payload_hex: "010300000002C40B"
  - action: "expect"
    endpoint: 2
    timeout_ms: 500
    match_hex: "010300000002C40B"
  - action: "set_pins"
    endpoint: 1
    rts: true
  - action: "assert_pins"
    endpoint: 1
    timeout_ms: 500
    rts: true
  - action: "assert_metrics"
    min_packets: 1
```

Unknown keys at the top level (`version`, `name`, `description`, `endpoints`,
`routes`, `capture`, `steps`) and inside endpoint/route/capture/fault mappings
are rejected by `serde(deny_unknown_fields)`. Unknown keys inside a step are
rejected by an explicit pre-parse key check, because serde cannot apply
`deny_unknown_fields` to an internally tagged enum.

## Endpoints

| Type | Behaviour | Required fields |
| :--- | :--- | :--- |
| `mock` | In-memory endpoint driven by scenario steps (`send` routes into it, `expect` reads from it). | `id`, `name`, `type` |
| `script` | Runs a Lua script via `omniport-script`. `on_rx(payload, pins)` receives routed frames; `serial.send(...)` queues responses; `serial.set_pins{...}` changes pins. | `id`, `name`, `type`, non-empty `script_path` |

`tcp_server`, `tcp_client` and `udp` are **rejected at load time**: the scenario
runner has no network bridge, and previously those endpoints were accepted and
silently skipped (a green test that tested nothing). Network endpoints are owned
by `omniportd`, not by the CI scenario runner. Likewise `address` is rejected,
`script_path`/`fuel_limit` are rejected on mock endpoints, endpoint ids must be
unique, and every step/route endpoint must be declared.

## Routes and faults

Routes are installed through `omniport_core::topology::Topology`, which allocates
canonical route ids. The optional YAML `id` field is therefore **rejected when
non-zero** instead of being parsed and ignored; remove it to let the core
allocate. Sources/targets must exist, targets must be non-empty and unique, and
a route cannot target its own source.

Fault profiles map onto `FaultConfig` with strict validation (`validate.rs` and
`convert_fault_config`):

- `drop_rate` is only meaningful as `0.0` (no drops) or `1.0` (full disconnect);
  any fraction in between is rejected because the fault injector has no
  per-frame Bernoulli drop.
- `bit_error_rate` ∈ `0.0..=1.0`; `mean_latency_ms`/`jitter_stddev_ms` are finite
  and non-negative (mapped to the Gaussian jitter mean/std dev).
- `corrupt_nth_crc` corrupts every Nth CRC-protected frame when > 0.

## Steps

| Action | Fields | Semantics |
| :--- | :--- | :--- |
| `sleep` | `duration_ms` | Pause the scenario. |
| `send` | `endpoint`, exactly one of `payload_hex` / `payload_text` | Route a frame from the endpoint; increments the sent counter. |
| `expect` | `endpoint`, `timeout_ms` (default 1000), at least one of `match_hex` / `match_text` / `min_bytes` | Wait for a routed frame matching all supplied criteria; increments the received counter. |
| `set_pins` | `endpoint`, at least one of `rts` / `dtr` / `brk` | Update the runner's tracked pin state and route a `PIN_CHANGE` event. |
| `assert_pins` | `endpoint`, `timeout_ms`, at least one of `rts`/`cts`/`dtr`/`dsr`/`dcd`/`ri` | Poll the tracked state every 10 ms until all expected pins match or the deadline expires; failures report the waited time plus expected/actual pins. |
| `assert_metrics` | at least one of `min_packets` / `max_drops` | Check counters; `max_drops` is computed as `sent - received`. |

Pin tracking: `set_pins` updates the asserted endpoint's state immediately. A
script endpoint's pin snapshot is also mirrored whenever it emits an event, so
`assert_pins` observes pins changed asynchronously by Lua (for example inside
`on_rx`) instead of only local `set_pins` calls.

Steps that cannot fail (empty `assert_pins`, empty `assert_metrics`,
`set_pins` with no flags, `send` with both/neither payload form, `expect` with no
criteria) are rejected at load time, as is an undeclared `endpoint` reference.

## Report

`omniport run <file> --json` prints a `ScenarioReport`:
`{ success, scenario_name, total_steps, passed_steps, failed_steps, duration_ms,
steps: [{ index, action, description, duration_ms, passed, error? }],
pcapng_output?, error? }`. A failing step records its error and stops the run.
