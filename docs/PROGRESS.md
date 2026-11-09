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
- [ ] **Step 3.2:** Build the 60 FPS Virtualized HTML5 Canvas Hex/ASCII Monitor and synchronized DTR/RTS/DCD Control-Pin Logic Strip.
- [ ] **Step 3.3:** Build the `@xyflow/react` Visual Topology Canvas and `uPlot` 60 Hz real-time telemetry strip chart.

## Phase 4: Simulation, RFC 2217 Networking & CI Test Harness
- [ ] **Step 4.1:** Implement `omniport-script` sandboxed Lua 5.4 runtime (`mlua` with instruction fuel limits and pin/timer hooks) [FR-403, NFR-105].
- [ ] **Step 4.2:** Implement `omniport-net` TCP/UDP/mTLS (`rustls`) and RFC 2217 control-pin synchronization.
- [ ] **Step 4.3:** Build declarative `.omniport.yaml` CI scenario runner in `omniport-cli`.
