# Software Requirements Specification (SRS)

## 1. Functional Requirements
- **FR-101 (Dynamic Lifecycle):** Create, configure, and destroy virtual serial ports on Windows, Linux, and macOS without rebooting.
- **FR-102 (POSIX Baud Throttling):** Allocate PTY pairs, maintain symlinks, poll slave `termios`, and enforce token-bucket baud throttling: $R_{\text{bytes}} = B / (1 + D + P + S)$.
- **FR-103 (Linux CUSE Pins):** Emulate RTS, CTS, DTR, DSR, DCD, RI, and BREAK via `/dev/cuse` ioctl interception (`TIOCMGET`, `TIOCMSET`, `TIOCMIWAIT`).
- **FR-104 (Windows Backend):** Manage persistent `COMx` pairs programmatically via `SetupAPI` and `DeviceIoControl`.
- **FR-201 (Stream Packetizer):** Frame raw byte streams via Raw, Delimiter, COBS, SLIP, Length-Field, and Inter-Character Idle-Gap rules.
- **FR-202 (Routing Matrix):** Support $1:1$, $1:N$ splitting, and $N:1$ merging across virtual, physical, and network endpoints.
- **FR-203 (Atomic Merging):** Serialize complete framed packets atomically during $N:1$ merges to prevent byte interleaving.
- **FR-301 (Timing Faults):** Inject fixed delay, Box-Muller Gaussian jitter, bandwidth limits, and disconnects on any routing edge.
- **FR-302 (Corruption Faults):** Inject uniform random bit flips, Gilbert-Elliott burst errors, and targeted Nth-frame CRC corruption.
- **FR-401 (Dual-Mmap Capture):** Record events into `session.idx` (24-byte stride) and `session.dat` with $O(1)$ viewport seeking.
- **FR-402 (C-Struct Dissector):** Parse packed C struct definitions at runtime and extract scaled engineering values.
- **FR-403 (Lua Simulator):** Execute sandboxed Lua 5.4 device scripts with instruction fuel limits.
- **FR-404 (Wireshark Extcap):** Stream live framed packets to Wireshark via PCAPNG named pipes.

## 2. Non-Functional Requirements
- **NFR-101:** Local virtual pair $P_{99}$ forwarding latency < 500 microseconds for 64-byte frames.
- **NFR-102:** Aggregate throughput >= 12 Mbps across 32 active port pairs with zero dropped bytes.
- **NFR-103:** Desktop UI RAM usage < 250 MB during a 10 GB capture session.
- **NFR-104:** 60 FPS (16.6 ms) UI frame budget for Hex Canvas and telemetry plots.
- **NFR-105:** Runaway Lua scripts terminated within 1 ms via instruction count hooks.
