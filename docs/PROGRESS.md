# OmniPort Step-by-Step Implementation Tracker

Use `/step-runner` in Antigravity 2.0 to execute each step sequentially.

## Phase 1: Core Engine, Framing, Capture & Dissector Foundation
- [x] **Step 1.0:** Bootstrap Cargo workspace, Antigravity 2.0 rules/skills, and baseline traits (`omniport-core`, `omniport-capture`, `omniport-dissect`).
- [x] **Step 1.1:** Complete `StreamPacketizer` in `omniport-core` (add SLIP, Idle-Gap timer, and CRC8/16/32 validation + `proptest` fuzzing) [FR-201].
- [x] **Step 1.2:** Implement Lock-Free Routing Graph (`1:1`, `1:N` Splitter, `N:1` Atomic Frame Merger) and Fault Injector (Gaussian jitter, Gilbert-Elliott burst errors) in `omniport-core` [FR-202, FR-203, FR-301, FR-302].
- [x] **Step 1.3:** Complete `omniport-capture` PCAPNG file writer and live Wireshark `extcap` named pipe streamer [FR-401, FR-404].
- [x] **Step 1.4:** Build `omniport-dissect` C-struct parser (`struct __attribute__((packed))` lexer + bitfield extractor) [FR-402].

## Phase 2: OS Virtual Serial Port Backends & Headless Daemon
- [x] **Step 2.1:** Implement `omniport-hal-posix` Tier-1 PTY manager (`/dev/ptmx`, symlink manager, `termios` poller, and Token-Bucket software baud throttler) [FR-101, FR-102].
- [x] **Step 2.2:** Implement `omniport-hal-posix` Tier-2 Linux CUSE (`/dev/cuse`) character device for user-space `TIOCMGET`/`TIOCMSET`/`TIOCMIWAIT` pin emulation [FR-103].
- [x] **Step 2.3:** Implement `omniport-hal-windows` Overlapped Win32 UART and `com0com` / `SetupAPI` programmatic virtual pair controller [FR-104].
- [x] **Step 2.4:** Implement `omniport-proto` gRPC schema, `omniportd` background service, and `omniport-cli` headless commands.

## Phase 3: Tauri v2 + React 19 Workbench GUI
- [x] **Step 3.1:** Wire Tauri v2 `fetch_viewport_rows` raw binary IPC command to `MmapCaptureEngine` and build the TypeScript `DataView` decoder [NFR-103, NFR-104].
- [x] **Step 3.2:** Build the 60 FPS Virtualized HTML5 Canvas Hex/ASCII Monitor and synchronized DTR/RTS/DCD Control-Pin Logic Strip.
- [x] **Step 3.3:** Build the `@xyflow/react` Visual Topology Canvas and `uPlot` 60 Hz real-time telemetry strip chart.

## Phase 4: Simulation, RFC 2217 Networking & CI Test Harness
- [x] **Step 4.1:** Implement `omniport-script` sandboxed Lua 5.4 runtime (`mlua` with instruction fuel limits and pin/timer hooks) [FR-403, NFR-105].
- [x] **Step 4.2:** Implement `omniport-net` TCP/UDP/mTLS (`rustls`) and RFC 2217 control-pin synchronization.
- [x] **Step 4.3:** Build declarative `.omniport.yaml` CI scenario runner in `omniport-cli`.

## Phase 5: Audit, Hardening & Architecture Deepening
- [x] **Step 5.0:** Baseline verification: `cargo check`, `cargo test --workspace`, `npm test`, `tsc`, `vite build`; fix pre-existing clippy blocker.
- [x] **Step 5.1:** Data-plane crate audit (`omniport-core`, `omniport-capture`, `omniport-dissect`): adversarial-input, framing/CRC, routing, fault-model, pcapng and parser bug fixes with regression tests.
- [x] **Step 5.2:** Service/platform crate audit (`omniport-net`, `omniport-script`, `omniport-proto`, `omniportd`, `omniport-cli`, `omniport-hal-*`): protocol, sandbox, scenario-runner and backend bug fixes with regression tests.
- [x] **Step 5.3:** Desktop UI audit + browser GUI test pass (web-gui-tester methodology) + design-engineering polish per the `emil-design-eng` review.
- [x] **Step 5.4:** Architecture deepening per the `improve-codebase-architecture` review: `ViewportBatch` wire-format module, golden cross-language fixture (`fixtures/viewport_v1.json`), ICD bit tables, `GLOSSARY.md`, removal of shallow modules.
- [x] **Step 5.5:** Final verification slice (`cargo fmt --check`, `cargo clippy -D warnings`, `cargo test --workspace`, Tauri host tests, `npm test`, `tsc`, `vite build`) and documentation/tracker reconciliation.

## Phase 6: Post-Audit Hardening & Daemon-First Follow-Through

Plan produced from the 2026-11-10 audit (`docs/AUDIT_2026-11-10.md`); architecture candidates ranked in the review report. Execute in order unless a step says otherwise.

- [x] **Step 6.1 — Land the audit in reviewable commits.** Split the 90-path working tree into lane-scoped commits, each self-consistent: (1) `fix(core)` framing/fault/routing/throttler defects, (2) `feat(capture)` ViewportBatch contract + fixture + capture/dissect hardening + Tauri host adapter + GLOSSARY, (3) `fix(net,script,daemon,cli,hal)` services/platform defects, (4) `feat(desktop)` viewportSession + GUI/design fixes, (5) `docs` audit report, Phase 6 plan and agent-reference path fixes. Re-run `cargo test --workspace`, `cargo clippy -D warnings`, `npm test`, `tsc` before the final commit. Do not push.
- [x] **Step 6.2 — Real Tauri shell smoke test.** `npm run tauri dev` click-through and a release `tauri build`: confirm the packaged window is not blank, that `fetch_viewport_rows` + `capture_status` IPC work through the real WebView (the browser suite only exercised the demo adapter), and that a killed/failed host surfaces the CAPTURE DISCONNECTED banner. Save evidence as `gui-test-screenshots/tauri_*.png` and check the release binary starts without the debug-only demo seed.
- [ ] **Step 6.3 — CI pipeline.** Add a workflow (or extend the existing one) running: `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, the Tauri host tests (`apps/omniport-desktop/src-tauri`), `npm test`, `npx tsc --noEmit`, `npx vite build`. Cache cargo/npm. The golden viewport fixture contract must run on every push.
- [ ] **Step 6.4 — `CaptureStore` seam (ADR-001).** Define one `CaptureStore` interface at the Tauri seam with two adapters: `DaemonCaptureStore` (over `omniport-proto`) and `FixtureCaptureStore` (explicit dev/demo only). Move the seeded demo frames behind the fixture adapter, select the adapter once in `App`/host setup, delete the remaining fake-data path from shipping code, and prove the daemon-backed path with an integration test. This is the step that makes the GUI a real daemon client.
- [ ] **Step 6.5 — `Topology` deep module.** Unify the three route shapes (`omniportd::RouteEntry`, `omniport-core` directed edges, scenario/TS edge models) behind `Topology::apply(Request) -> Result<Response>`; validate routes against live endpoints and keep ids/lifecycle behind the seam. Resolve the protocol truth: either generate one `.proto` (introducing tonic) or delete both and document the length-prefixed JSON transport in ICD §3. The stale root `proto/omniport.proto` must not survive this step.
- [ ] **Step 6.6 — Platform / HIL hardening.** com0com pair removal by real pair index (and loud failure when the driver is absent); POSIX PTY disconnect detection (`EIO` on master close, `ptsname_r` error handling); real CUSE `/dev/cuse` FUSE wiring with correct `TCGETS`/`TIOCGICOUNT` struct sizes; RFC 2217 negotiation reply state machine; mlua memory limit for C-function allocations (`string.rep`, `table.concat`). Items that need hardware/driver environments are verified on those hosts and recorded in `VV_RTM`.
- [x] **Step 6.7 — Version the engineering specifications.** Move `SRS.md`, `ICD.md` and `VV_RTM.md` into `docs/`; split the monolithic ADR list into immutable files under `docs/adr/` (0001–0007); remove the `internal_docs/` ignore and directory; add a `docs/README.md` index and an `docs/adr/README.md` register; update every reference (`.agents/` rules and skills, fixture generator and README, source doc comments). Specs become reviewable and traceable in the same commits as the code they constrain.
- [ ] **Step 6.8 — Amend decisions and commit the transport spec.** Amend ADR-0003 to scope its zero-copy claim (routing/tapping yes, viewport serialization copies); resolve the stale root `proto/omniport.proto` (delete it, or generate the design-only proto that mirrors `types.rs`); add the current daemon control-plane transport (4-byte length-prefixed JSON over TCP) to the committed spec (ICD §3) so ADR-0001's gRPC phrasing is not mistaken for implemented behaviour.


