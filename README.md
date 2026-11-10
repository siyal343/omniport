# OmniPort

**Virtual serial port router, stream packetizer, packet dissector, and HIL fault-injection
platform for Windows and Linux.**

[![CI](https://github.com/siyal343/omniport/actions/workflows/ci.yml/badge.svg)](https://github.com/siyal343/omniport/actions/workflows/ci.yml)

OmniPort gives firmware and protocol teams a reproducible serial lab without physical
hardware: create virtual serial port pairs (or POSIX PTYs / Linux CUSE devices), route
frames between them, stream the raw bytes into a Wireshark-compatible capture, decode
packed C structs in a sandboxed Lua VM, and inject realistic faults (jitter, bit errors,
disconnects, CRC corruption) — all driven from a headless daemon, a declarative CI
scenario, and a Tauri/React desktop workbench.

## Features

- **Virtual serial endpoints:** virtual pairs, POSIX PTYs (Linux/macOS), Windows
  `com0com`/UMDF2 pairs, Linux CUSE modem-control devices, TCP/RFC 2217 network serial.
- **Zero-copy routing:** `bytes::Bytes` frames are fanned out through a persistent
  radix-trie routing graph with bounded queues; payloads are only copied at mutation and
  serialization boundaries (see ADR-0009).
- **Dual-mmap capture:** `session.idx` (24-byte strides) + `session.dat` give O(1)
  viewport reads over multi-gigabyte captures; the GUI never receives per-packet JSON.
- **Fault injection:** Gaussian latency/jitter, per-frame bit errors, burst errors,
  disconnects, CRC corruption, rate limiting.
- **Lua device simulation:** sandboxed Lua 5.4 scripts with instruction fuel, memory
  limits, and packet framing rules (SLIP/COBS/idle-gap/CRC).
- **Runtime C-struct dissector:** decode packed C structs from an ICD without recompiling.
- **Wireshark extcap + PCAPNG export** for offline analysis.
- **Desktop workbench:** virtualized hex/ASCII monitor, logic strip, 60 FPS telemetry,
  live topology editor backed by the daemon's control plane.
- **CI-native:** `omniport run scenario.yaml --json` drives a full multi-endpoint topology
  with assertions, metrics, and PCAPNG output — see [docs/SCENARIO.md](docs/SCENARIO.md).

## Architecture

```
┌──────────────┐   JSON/TCP control plane (ADR-0008)   ┌────────────────────┐
│ omniport-cli │◄────────────────────────────────────►│                    │
│ Tauri desktop│                                       │     omniportd      │
└──────────────┘◄──── binary viewport batches (ICD §2) │  (headless daemon) │
                                                       │                    │
   endpoints ──► routing graph ──► fault injection ────┤ capture store      │
   (PTY, pairs,   (trie RCU,        (latency/BER/      │ (dual mmap)        │
    TCP, CUSE)     bounded queues)   corrupt/drop)     └────────────────────┘
```

The daemon owns endpoints, routes, capture, and scripts; clients are stateless. Closing
the GUI never drops virtual serial ports. See [docs/adr/](docs/adr/) for the decisions
behind this design.

## Repository layout

| Path | Contents |
| :--- | :--- |
| `crates/omniport-core` | Framing, routing graph, topology, fault injection, endpoint traits |
| `crates/omniport-capture` | Dual-mmap capture store, PCAPNG, Wireshark extcap, ICD §2 viewport codec |
| `crates/omniport-dissect` | Runtime packed C-struct dissector |
| `crates/omniport-script` | Sandboxed Lua VM + script endpoints |
| `crates/omniport-net` | TCP / RFC 2217 serial-over-network endpoints |
| `crates/omniport-hal-posix` | POSIX PTY and Linux CUSE backends |
| `crates/omniport-hal-windows` | Windows COM / `com0com` backends |
| `crates/omniport-proto` | Control-plane wire types + length-prefixed JSON codec |
| `crates/omniportd` | Headless daemon + RPC server |
| `crates/omniport-cli` | CLI and declarative scenario runner |
| `apps/omniport-desktop` | Tauri v2 + React 19 workbench (Rust host in `src-tauri/`) |
| `docs/` | SRS, ICD, VV_RTM, ADRs, progress tracker, audit, scenario/release guides |
| `fixtures/` | Frozen ICD §2 cross-language contract fixture |
| `.omniport.yaml` | Reference CI scenario |

## Quick start

Prerequisites: Rust stable (edition 2024), Node.js 22 for the desktop app, and the
Windows WebView2 runtime on Windows.

```powershell
# Headless daemon (loopback control plane on 127.0.0.1:50051)
cargo run -p omniportd

# Run the reference scenario end-to-end (JSON report, PCAPNG capture)
cargo run -p omniport-cli -- run .omniport.yaml --json

# Desktop workbench (dev mode; expects a daemon for live data)
cd apps/omniport-desktop
npm ci
npx tauri dev
```

End-to-end verification:

```powershell
cargo test --workspace          # Rust workspace (Windows; see CI for the unix matrix)
cargo test -p omniportd         # daemon + control-plane test suites
cd apps/omniport-desktop; npm test; npx tsc --noEmit; npx vite build
cargo test --release -p omniport-core --test nfr_perf -- --ignored --nocapture
```

## Verification and CI

GitHub Actions runs six jobs on every push: Rust workspace (Windows), Rust workspace
(Ubuntu/unix backends), desktop host + frontend, release-mode performance/NFR harnesses,
supply-chain audit + license policy (`cargo audit` / `cargo deny`, see `deny.toml`), and
Windows installer build + packaged-app smoke test. Performance thresholds and recorded
baselines live in [docs/VV_RTM.md](docs/VV_RTM.md) Step 7.4.

## Documentation

- [docs/README.md](docs/README.md) — documentation index
- [docs/PROGRESS.md](docs/PROGRESS.md) — implementation tracker (single source of truth)
- [docs/AUDIT_2026-11-10.md](docs/AUDIT_2026-11-10.md) — audit snapshot and known gaps
- [docs/SRS.md](docs/SRS.md) / [docs/ICD.md](docs/ICD.md) — requirements and interfaces
- [docs/SCENARIO.md](docs/SCENARIO.md) — declarative scenario contract
- [docs/RELEASE.md](docs/RELEASE.md) — installer build, verification, signing follow-ups
- [AGENTS.md](AGENTS.md) — operating manual for contributors (human or agent)
- [GLOSSARY.md](GLOSSARY.md) — domain vocabulary

## Platform support

Windows 10/11 is the primary target (com0com/UMDF2, WebView2). Linux supports PTY and
CUSE backends and is exercised in CI; macOS support is not yet verified. Live HAL
verification on target hardware remains tracked as HIL-pending in the RTM.

## License

Apache-2.0 — see [LICENSE](LICENSE). The decision is recorded in
[ADR-0006](docs/adr/0006-apache-2-0-monorepo-license.md).
