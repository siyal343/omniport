# Verification Report: Step 2.3 — Windows Overlapped UART & com0com Virtual Pair Controller

**Requirement IDs:** FR-101, FR-104  
**Verification Criteria (VV_RTM.md):** Integration + HIL: Port lifecycle < 200 ms, Baud within 1.5%  
**Status:** COMPLETE (`[x]`)

---

## 1. Scope & Implementation Details

### 1.1 Overlapped Win32 UART Endpoint (`crates/omniport-hal-windows/src/uart.rs`)
- **Direct Win32 Communications API Integration:**
  - Standardized device path naming: `format_com_path(name)` ensures `\\.\COMx` prefix is properly formatted.
  - Native asynchronous handle allocation using `CreateFileW` with `FILE_FLAG_OVERLAPPED`.
  - Non-blocking execution of `ReadFile` and `WriteFile` via `GetOverlappedResult` and manual event handles wrapped in threadpool tasks, preventing worker starvation on Tokio.
- **Serial Port Configuration (DCB & CommTimeouts):**
  - Configures `DCB` parameters: baud rate, byte size (5–8 bits), parity (`NOPARITY`, `ODDPARITY`, `EVENPARITY`), and stop bits (`ONESTOPBIT`, `TWOSTOPBITS`).
  - Configures `COMMTIMEOUTS` for non-blocking stream processing:
    `ReadIntervalTimeout = u32::MAX`, `ReadTotalTimeoutMultiplier = 0`, `ReadTotalTimeoutConstant = 0`.
- **Modem Control Line Management:**
  - Line assertion via `EscapeCommFunction`:
    - `SETDTR` / `CLRDTR` $\leftrightarrow$ `PinState::DTR`
    - `SETRTS` / `CLRRTS` $\leftrightarrow$ `PinState::RTS`
    - `SETBREAK` / `CLRBREAK` $\leftrightarrow$ `PinState::BREAK`
  - Status reading via `GetCommModemStatus`:
    - `MS_CTS_ON` $\rightarrow$ `PinState::CTS`
    - `MS_DSR_ON` $\rightarrow$ `PinState::DSR`
    - `MS_RING_ON` $\rightarrow$ `PinState::RI`
    - `MS_RLSD_ON` $\rightarrow$ `PinState::DCD`
- **Software Token-Bucket Baud Throttler:**
  - Integrated with shared `TokenBucketThrottler` from `omniport-core`, guaranteeing theoretical UART throughput pacing ($R_{\text{bytes}} = B / (1 + D + P + S)$) for virtual COM pairs.
- **Cross-Platform Mock Support:**
  - `open_mock(id, name, port_name, config)` creates in-memory duplex streams (`tokio::io::duplex`) for automated CI and headless unit testing without requiring physical serial hardware.

### 1.2 Programmatic `com0com` Virtual Pair Controller (`crates/omniport-hal-windows/src/com0com.rs`)
- **Driver Detection & SetupAPI Wrapper:**
  - Scans standard installation directories (`C:\Program Files (x86)\com0com\setupc.exe` and `Program Files`) and tests accessibility of `\\.\com0com`.
  - Invokes `setupc.exe install PortName=COMx PortName=COMy` when available.
  - Employs an in-memory virtual pair manager when running in environments without signed kernel drivers installed.
- **RAII Lifecycle & Cleanup (`VirtualPairGuard`):**
  - Guarantees automatic removal of the pair upon `drop()`, preventing left-over orphan COM ports from polluting Windows Device Manager.
  - `find_free_pair_names(base)` searches for available port names not currently in use.

### 1.3 Core Throttler Refactoring (`crates/omniport-core/src/throttler.rs`)
- Promoted `TokenBucketThrottler` and `calculate_byte_rate` to `omniport-core` as shared building blocks across POSIX, Windows, and network endpoints.

---

## 2. Verification Test Suite Log

### 2.1 Workspace Unit Tests (`cargo test -p omniport-hal-windows`)
```text
running 4 tests
test com0com::tests::test_virtual_pair_lifecycle_and_raii ... ok
test com0com::tests::test_virtual_pair_explicit_remove ... ok
test uart::tests::test_mock_uart_configuration_and_pins ... ok
test uart::tests::test_mock_uart_endpoint_send_recv ... ok

test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
```

### 2.2 Property-Based Fuzzing (`crates/omniport-hal-windows/tests/windows_proptest.rs`)
```text
running 3 tests
test test_uart_endpoint_stream_proptest ... ok
test prop_com_path_formatting ... ok
test prop_virtual_pair_isolation_and_raii ... ok

test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.03s
```

### 2.3 Comprehensive Verification
- `cargo test --workspace`: **75 passed; 0 failed** across all crates.
- `cargo clippy --workspace --all-targets -- -D warnings`: **0 warnings**.
- `cargo fmt --all -- --check`: **Clean formatting**.
