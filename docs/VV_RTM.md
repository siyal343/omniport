# Verification & Validation Plan & Requirements Traceability Matrix

| Req ID | Module | Verification Method | Target Criteria |
| :--- | :--- | :--- | :--- |
| FR-101..104 | `omniport-hal-*` | Integration + HIL | Port lifecycle < 200 ms, Baud within 1.5% |
| FR-201..203 | `omniport-core` | Property Fuzzing | 100% frame recovery, zero merged byte interleaving |
| FR-301..302 | `omniport-core` | Statistical Test | Jitter and BER match configured parameters |
| FR-401 | `omniport-capture` | Benchmark + Unit | $O(1)$ viewport seek < 100 us at 50M packets |
| FR-402 | `omniport-dissect` | Unit + Fuzz Test | Decoded fields match packed C reference |
| FR-403, NFR-105 | `omniport-script` | Unit + Property Fuzzing | Instruction fuel abort <= 1 ms; bounded VM heap & `serial.send` |
| NFR-101 | `omniport-core` | Benchmark (release, `perf` CI job) | Loopback 64-byte forwarding p99 < 500 us |
| NFR-102 | `omniport-core` | Benchmark (release, `perf` CI job) | >= 12 Mbps across 32 pairs, zero dropped bytes |
| NFR-103 | `omniport-capture` | Memory harness (release, `perf` CI job) | Desktop RSS < 250 MB while a 10 GB capture is mapped |
| NFR-104 | `omniport-desktop` | Frame-budget test (`npm test`) | Per-frame data-path p95 <= 16.6 ms (60 FPS) |

## Step 7.4 Performance / NFR Verification Status

The `perf` CI job (`.github/workflows/ci.yml`, `windows-latest`, release profile) runs the
Rust harnesses explicitly with `--ignored --nocapture`; the desktop frame-budget test runs
inside the `desktop` job's `npm test`. Every harness prints the measured value next to its
target before asserting, so a failure shows the regression.

| Req ID | Harness | Threshold | Recorded baseline (2026-11-11, Windows 10 Pro, rustc 1.99.0, release) |
| :--- | :--- | :--- | :--- |
| NFR-101 | `crates/omniport-core/tests/nfr_perf.rs` | p99 < 500 us for 64-byte frames | p50 0.20 us, p99 0.30 us, max 2.03 ms outlier over 20,000 samples |
| NFR-102 | `crates/omniport-core/tests/nfr_perf.rs` | >= 12 Mbps over 32 pairs, zero dropped bytes | 34.6 Gbps (65,536,000 bytes / 64,000 frames in 15.17 ms), 0 dropped |
| NFR-103 | `crates/omniport-capture/tests/nfr_memory.rs` | RSS < 250 MB with a 10 GiB capture mapped | RSS 20.7 MiB after mapping 14,080 MiB (10 GiB data + ~3.9 GiB index); delta 16.9 MiB after 200,000 appended frames |
| NFR-104 | `apps/omniport-desktop/src/perf/frameBudget.test.js` | p95 data-path frame cost <= 16.6 ms | avg 0.263 ms, p95 0.522 ms, max 3.953 ms over 240 frames (decode 200 rows + hex draw-call emission + telemetry ingest) |

Notes:

- NFR-103 creates sparse capture files, so CI maps the same virtual footprint as a real 10 GB
  session without writing 10 GB; 200,000 appended frames exercise the used pages only.
- NFR-104 is a deterministic CPU-data-path budget (decode + canvas call emission + telemetry
  ingest). GPU/compositor frame pacing is not measurable in headless CI; the end-to-end
  60 FPS display claim remains covered by the manual WebView2 pass (`docs/AUDIT_2026-11-10.md` §5).
- The first CI `perf` run re-records these numbers on `windows-latest`; baselines above come from
  the local release run used to validate the harnesses.

## Step 6.6 Platform & Protocol Verification Status

The table below distinguishes checks **Verified** in the cross-platform Rust test suite from **HIL Pending** items that require target OS kernels or hardware drivers (cannot compile or execute live ioctls/syscalls on a Windows-only CI/dev host without `com0com` installed).

| Req ID | Capability | Automated Verification (Verified) | Target-Environment HIL (HIL Pending) | Status |
| :--- | :--- | :--- | :--- | :--- |
| FR-104 | `com0com` pair lifecycle (`crates/omniport-hal-windows/src/com0com.rs`) | Missing-driver failure (`Com0ComDriverNotFound`), port name/conflict validation, `CNCA<N>`/`CNCB<N>` pair-index parsing, `setupc` stub lifecycle & remove-error propagation (`cargo test -p omniport-hal-windows`) | Real `setupc.exe` + signed `com0com.sys` kernel pair creation, live Win32 overlapped COM I/O (`CreateFile`/`DeviceIoControl`), and `remove <pair_index>` cleanup on a host with `com0com` installed | Unit Verified / Driver HIL Pending |
| FR-101, FR-102 | POSIX PTY disconnect & `ptsname_r` (`crates/omniport-hal-posix/src/pty.rs`) | `ptsname_error` direct-errno vs `-1`+`errno` decoding, mock slave disconnect `UnexpectedEof`, symlink guard & token-bucket timing proptests (`cargo test -p omniport-hal-posix`) | Compilation and execution of `#[cfg(unix)]` `/dev/ptmx` open, live `/dev/pts/*` slave close surfacing `EIO`/`EOF` (`test_pty_endpoint_detects_slave_disconnect`), and measured `±1.5%` baud throughput on Linux/macOS | Mock/Unit Verified / POSIX HIL Pending |
| FR-103 | Linux `/dev/cuse` FUSE wiring & UAPI struct layouts (`crates/omniport-hal-posix/src/cuse.rs`) | UAPI sizes verified (`KernelTermios` 36 B, `termios2` 44 B + `BOTHER`, `SerialCounters` 80 B), `CuseFuseSession` state machine (`CUSE_INIT`, `FUSE_OPEN`, `FUSE_IOCTL_RETRY`, `TCGETS`/`TCSETS`, `FUSE_READ`/`FUSE_WRITE`, `-EAGAIN`), non-Linux loud failure (`OpenCuse`), `TIOCMIWAIT` mock wakeup (`cargo test -p omniport-hal-posix`) | Compilation of `#[cfg(target_os = "linux")] mod linux_cuse` and live `/dev/cuse` char device registration, `termios`/`TIOCM*`/`TIOCGICOUNT`/`TIOCMIWAIT` syscalls from C/`pyserial` against a Linux kernel with `cuse` loaded | Protocol & ABI Verified / Linux Kernel HIL Pending |
| FR-202 | RFC 2217 option negotiation (`crates/omniport-net/src/rfc2217.rs`, `tcp.rs`) | RFC 1143 `Rfc2217Negotiator` unit tests (`COM_PORT_OPTION`, `TRANSMIT_BINARY`, `SUPPRESS_GO_AHEAD`, unknown option `WONT`/`DONT`), live TCP `TcpEndpoint` negotiation reply & peer-to-peer convergence tests (`cargo test -p omniport-net`) | Interoperability smoke test against external `ser2net` / `pyserial` `rfc2217://` servers | Verified (Protocol + TCP Loopback) |
| FR-403, NFR-105 | Lua VM memory & bridge allocation limits (`crates/omniport-script/src/engine.rs`, `bindings.rs`) | `lua.set_memory_limit` enforcement (`MemoryExceeded` across `string.rep`, `table.concat`, top-level eval, and error recovery GC), `serial.send` 1 MiB frame & dense-table bounds (`cargo test -p omniport-script`) | — | Verified |

