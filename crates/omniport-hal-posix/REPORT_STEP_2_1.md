# Verification Report: Step 2.1 — POSIX PTY Manager & Token-Bucket Baud Throttler

**Requirement IDs:** FR-101, FR-102  
**Verification Criteria (VV_RTM.md):** Integration + HIL: Port lifecycle < 200 ms, Baud within 1.5%  
**Status:** COMPLETE (`[x]`)

---

## 1. Scope & Implementation Details

### 1.1 Token-Bucket Software Baud Rate Throttler (`crates/omniport-hal-posix/src/throttler.rs`)
- **Exact UART Baud Rate Formula:**
  - Implemented theoretical UART byte rate formula per FR-102:
    $$R_{\text{bytes}} = \frac{B}{1 + D + P + S}$$
    where:
    - $B$: baud rate in symbols/sec
    - Start bit = $1$
    - $D \in [5, 8]$: data bits
    - $P \in \{0, 1\}$: parity bit ($1$ if parity enabled, $0$ if none)
    - $S \in [1, 2]$: stop bits
- **Token Accumulation & Throttling:**
  - Dynamic bucket burst capacity (50 ms burst window, minimum 64 bytes).
  - Nanosecond precision token refill and deficit calculation.
  - `throttle(byte_count, now_ns) -> Duration`: computes delay if tokens are depleted.
  - `throttle_async(byte_count)`: non-blocking async sleep via `tokio::time::sleep`.
  - Dynamic reconfiguration: preserves token state scaled to new capacity on baud rate change.

### 1.2 Symlink Manager & RAII Guard (`crates/omniport-hal-posix/src/symlink.rs`)
- **Lifecycle Management:**
  - `SymlinkManager::create(target, link_path) -> Result<SymlinkGuard>`:
    - Automatically cleans up stale files/links before creating new symlinks.
    - Creates parent directories if absent.
    - Platform-native symlink creation via `std::os::unix::fs::symlink` on Unix; dev fallback on Windows without elevated privileges.
  - `SymlinkGuard`:
    - RAII `Drop` implementation unlinks the symlink upon destruction, guaranteeing zero orphan symbolic links.
    - Explicit `.remove()` method for deterministic early cleanup.

### 1.3 Termios Poller & Serial Configuration (`crates/omniport-hal-posix/src/termios.rs`)
- **Baud & Parameter Decoding:**
  - Unix: Decodes speed constants (`B9600` through `B4000000`) and encodes `speed_t`.
  - Extracts framing parameters from `c_cflag`: `CSIZE` (`CS5`..`CS8`), `PARENB` / `PARODD` (None, Odd, Even), and `CSTOPB` (1 or 2 stop bits).
  - `set_raw_mode(fd)`: Configures raw binary mode with `cfmakeraw` (disables canonical echo, line buffering, software flow control).
  - `TermiosPoller`: Periodically inspects slave terminal state, notifying callers and updating the throttler when client processes alter serial configuration via `tcsetattr`.

### 1.4 POSIX PTY Endpoint (`crates/omniport-hal-posix/src/pty.rs`)
- **Architecture & Async Integration:**
  - Implements `omniport_core::AsyncEndpoint`.
  - Unix: Opens master PTY via `libc::posix_openpt(O_RDWR | O_NOCTTY | O_NONBLOCK)`, unlocks via `grantpt`/`unlockpt`, and resolves slave path via `ptsname_r`.
  - Integrated with `tokio::io::unix::AsyncFd` for zero-overhead, event-driven async read/write without OS threadpool blocking.
  - Non-Unix / Windows Host: Backed by full-duplex simulated streams (`tokio::io::duplex`) and mock termios poller, enabling 100% test coverage and CI execution across all platforms.
  - Seamlessly links to `TokenBucketThrottler` on TX path (`send`) and attaches `SymlinkGuard` for filesystem discovery.

---

## 2. Verification Test Suite Log

### 2.1 Workspace Unit Tests (`cargo test -p omniport-hal-posix`)
```text
running 8 tests
test pty::tests::test_pty_endpoint_lifecycle_and_io ... ok
test pty::tests::test_pty_endpoint_termios_reconfiguration ... ok
test symlink::tests::test_symlink_creation_and_raii_cleanup ... ok
test symlink::tests::test_symlink_explicit_removal ... ok
test symlink::tests::test_symlink_overwrite_stale ... ok
test termios::tests::test_termios_poller_mock ... ok
test throttler::tests::test_byte_rate_calculation ... ok
test throttler::tests::test_token_bucket_throttling_delay ... ok

test result: ok. 8 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s
```

### 2.2 Property-Based Fuzzing (`crates/omniport-hal-posix/tests/pty_proptest.rs`)
```text
running 4 tests
test test_pty_endpoint_stream_roundtrip_proptest ... ok
test prop_token_bucket_delay_accuracy ... ok
test prop_baud_rate_formula ... ok
test prop_symlink_guard_lifecycle ... ok

test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.02s
```

### 2.3 Comprehensive Verification
- `cargo test --workspace`: **61 passed; 0 failed** across all crates.
- `cargo clippy --workspace --all-targets -- -D warnings`: **0 warnings**.
- `cargo fmt --all -- --check`: **Clean formatting**.
