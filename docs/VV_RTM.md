# Verification & Validation Plan & Requirements Traceability Matrix

| Req ID | Module | Verification Method | Target Criteria |
| :--- | :--- | :--- | :--- |
| FR-101..104 | `omniport-hal-*` | Integration + HIL | Port lifecycle < 200 ms, Baud within 1.5% |
| FR-201..203 | `omniport-core` | Property Fuzzing | 100% frame recovery, zero merged byte interleaving |
| FR-301..302 | `omniport-core` | Statistical Test | Jitter and BER match configured parameters |
| FR-401 | `omniport-capture` | Benchmark + Unit | $O(1)$ viewport seek < 100 us at 50M packets |
| FR-402 | `omniport-dissect` | Unit + Fuzz Test | Decoded fields match packed C reference |
| FR-403, NFR-105 | `omniport-script` | Unit + Property Fuzzing | Instruction fuel abort <= 1 ms; bounded VM heap & `serial.send` |

## Step 6.6 Platform & Protocol Verification Status

The table below distinguishes checks **Verified** in the cross-platform Rust test suite from **HIL Pending** items that require target OS kernels or hardware drivers (cannot compile or execute live ioctls/syscalls on a Windows-only CI/dev host without `com0com` installed).

| Req ID | Capability | Automated Verification (Verified) | Target-Environment HIL (HIL Pending) | Status |
| :--- | :--- | :--- | :--- | :--- |
| FR-104 | `com0com` pair lifecycle (`crates/omniport-hal-windows/src/com0com.rs`) | Missing-driver failure (`Com0ComDriverNotFound`), port name/conflict validation, `CNCA<N>`/`CNCB<N>` pair-index parsing, `setupc` stub lifecycle & remove-error propagation (`cargo test -p omniport-hal-windows`) | Real `setupc.exe` + signed `com0com.sys` kernel pair creation, live Win32 overlapped COM I/O (`CreateFile`/`DeviceIoControl`), and `remove <pair_index>` cleanup on a host with `com0com` installed | Unit Verified / Driver HIL Pending |
| FR-101, FR-102 | POSIX PTY disconnect & `ptsname_r` (`crates/omniport-hal-posix/src/pty.rs`) | `ptsname_error` direct-errno vs `-1`+`errno` decoding, mock slave disconnect `UnexpectedEof`, symlink guard & token-bucket timing proptests (`cargo test -p omniport-hal-posix`) | Compilation and execution of `#[cfg(unix)]` `/dev/ptmx` open, live `/dev/pts/*` slave close surfacing `EIO`/`EOF` (`test_pty_endpoint_detects_slave_disconnect`), and measured `±1.5%` baud throughput on Linux/macOS | Mock/Unit Verified / POSIX HIL Pending |
| FR-103 | Linux `/dev/cuse` FUSE wiring & UAPI struct layouts (`crates/omniport-hal-posix/src/cuse.rs`) | UAPI sizes verified (`KernelTermios` 36 B, `termios2` 44 B + `BOTHER`, `SerialCounters` 80 B), `CuseFuseSession` state machine (`CUSE_INIT`, `FUSE_OPEN`, `FUSE_IOCTL_RETRY`, `TCGETS`/`TCSETS`, `FUSE_READ`/`FUSE_WRITE`, `-EAGAIN`), non-Linux loud failure (`OpenCuse`), `TIOCMIWAIT` mock wakeup (`cargo test -p omniport-hal-posix`) | Compilation of `#[cfg(target_os = "linux")] mod linux_cuse` and live `/dev/cuse` char device registration, `termios`/`TIOCM*`/`TIOCGICOUNT`/`TIOCMIWAIT` syscalls from C/`pyserial` against a Linux kernel with `cuse` loaded | Protocol & ABI Verified / Linux Kernel HIL Pending |
| FR-202 | RFC 2217 option negotiation (`crates/omniport-net/src/rfc2217.rs`, `tcp.rs`) | RFC 1143 `Rfc2217Negotiator` unit tests (`COM_PORT_OPTION`, `TRANSMIT_BINARY`, `SUPPRESS_GO_AHEAD`, unknown option `WONT`/`DONT`), live TCP `TcpEndpoint` negotiation reply & peer-to-peer convergence tests (`cargo test -p omniport-net`) | Interoperability smoke test against external `ser2net` / `pyserial` `rfc2217://` servers | Verified (Protocol + TCP Loopback) |
| FR-403, NFR-105 | Lua VM memory & bridge allocation limits (`crates/omniport-script/src/engine.rs`, `bindings.rs`) | `lua.set_memory_limit` enforcement (`MemoryExceeded` across `string.rep`, `table.concat`, top-level eval, and error recovery GC), `serial.send` 1 MiB frame & dense-table bounds (`cargo test -p omniport-script`) | — | Verified |

