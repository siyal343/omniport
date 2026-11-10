# OmniPort — Antigravity 2.0 Agent Operating Manual

OmniPort is a cross-platform Virtual Serial Port (VSP) router, stream packetizer,
zero-compile C-struct dissector, and HIL fault-injection platform built in Rust,
Tauri v2, and React 19.

## Core Architectural Invariants (NEVER VIOLATE)
1. **Daemon-First Separation:** `omniportd` runs as an independent background service.
   `omniport-desktop` (Tauri) and `omniport-cli` are stateless clients. Closing the GUI
   must never drop active virtual serial ports.
2. **Zero JSON on the High-Speed Data Path:** Never emit per-packet JSON events over
   Tauri IPC. Capture frames to the dual-mmap store (`omniport-capture`) and read
   only visible rows via raw binary `ArrayBuffer` slices (`fetch_viewport_rows`).
3. **Zero-Copy Routing:** Pass payloads between endpoints using `bytes::Bytes` inside
   `SerialEvent`. Allocate new buffers only when a Fault Injector or Lua script mutates data.
4. **Tiered OS Backends:** Use POSIX PTY + token-bucket baud throttling (and Linux CUSE for
   modem pins) in `omniport-hal-posix`, and `com0com` / UMDF2 in `omniport-hal-windows`.

## Verification Policy (GitHub Actions First)
- **Offload full-workspace verification to GitHub Actions:** Do NOT run full-workspace
  sweeps locally (`cargo check --workspace`, `cargo clippy --workspace`,
  `cargo test --workspace`, or full `apps/omniport-desktop` build/test matrices) when
  GitHub Actions CI is available, unless the user explicitly asks or a local run is
  required to reproduce a failing CI job.
- **Keep local checks crate-scoped:** During implementation, verify only touched crates
  and files locally (`cargo fmt --all`, `cargo check -p <crate>`,
  `cargo clippy -p <crate> --all-targets --locked -- -D warnings`,
  `cargo test -p <crate> --locked`, or targeted frontend tests via `npx vitest run <file>`).
- **CI Workflow & Monitoring:** `.github/workflows/ci.yml` executes the full Rust
  workspace (`windows-latest`), Tauri host (`apps/omniport-desktop/src-tauri`), and
  frontend checks automatically on `push` / `pull_request`. Monitor runs with
  `gh run watch` and inspect errors with `gh run view --log-failed`.

## Step-by-Step Execution Protocol
Always check `@[Progress Tracker](docs/PROGRESS.md)` before starting a new task.
Use the `/step-runner` skill to plan, implement, verify, and check off one step at a time.
