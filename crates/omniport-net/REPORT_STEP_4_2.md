# OmniPort Step 4.2 Verification Report: TCP/UDP/mTLS & RFC 2217 Control-Pin Synchronization

**Date:** 2026-11-10  
**Phase:** 4 — Simulation, RFC 2217 Networking & CI Test Harness  
**Step:** 4.2 — Implement `omniport-net` TCP/UDP/mTLS (`rustls`) and RFC 2217 control-pin synchronization [FR-202]  
**Status:** PASSED (Verification Complete)

---

## 1. Architectural Context & Requirements

OmniPort connects remote serial equipment, network serial servers (e.g. Moxa NPort, Linux `ser2net`, Lantronix), and secure cloud telemetry collectors over industrial networks:
- **FR-202 (Routing Matrix):** Support $1:1$, $1:N$ splitting, and $N:1$ merging across physical UARTs, virtual pairs, and networked endpoints.
- **RFC 2217 (Telnet Com Port Control Option):** Complete state machine for remote serial baud rate, framing, and hardware modem pin synchronization (DTR, RTS, CTS, DSR, DCD, RI, BREAK).
- **Mutual TLS (mTLS):** Cryptographically authenticated and encrypted transport using modern `rustls` and WebPKI X.509 certificates to protect serial streams across untrusted networks.
- **UDP Datagrams:** Ultra-low latency connectionless transport for high-rate telemetry broadcast.

---

## 2. Component Implementation Details

### 2.1 RFC 2217 Telnet Com Port Control Option (`rfc2217.rs`)
- **Streaming Parser & Codec (`Rfc2217Codec`):**
  - **RFC 854 Data Byte Escaping:** Escapes binary data byte `0xFF` as `[0xFF, 0xFF]`. Unescapes incoming streams transparently.
  - **Option Negotiation:** Handles `IAC DO COM_PORT_OPTION (44)` and `IAC WILL COM_PORT_OPTION (44)` handshakes.
  - **Subnegotiation Framing (`IAC SB 44 ... IAC SE`):**
    - `SET_BAUDRATE (1)` & `NOTIFY_BAUDRATE (101)`: 4-byte big-endian baud rate.
    - `SET_DATASIZE (2)` & `NOTIFY_DATASIZE (102)`: Data bits ($5 \dots 8$).
    - `SET_PARITY (3)` & `NOTIFY_PARITY (103)`: 1 = None, 2 = Odd, 3 = Even.
    - `SET_STOPSIZE (4)` & `NOTIFY_STOPSIZE (104)`: 1 = 1 stop bit, 2 = 2 stop bits.
    - `SET_CONTROL (5)`: Outbound modem pin control:
      - `8` / `9`: DTR ON / OFF
      - `11` / `12`: RTS ON / OFF
      - `5` / `6`: BREAK ON / OFF
    - `NOTIFY_MODEMSTATE (107)`: Inbound modem signal transition notification:
      - Bit 4 (`0x10`): CTS (Clear To Send)
      - Bit 5 (`0x20`): DSR (Data Set Ready)
      - Bit 6 (`0x40`): RI (Ring Indicator)
      - Bit 7 (`0x80`): DCD (Data Carrier Detect)
- **Modem State Synchronization:**
  - Incoming `NOTIFY_MODEMSTATE` packets update local `PinState` atomically and dispatch `FrameFlags::PIN_CHANGE` events downstream.
  - Outbound `set_pins()` invocations generate differential `SET_CONTROL` commands only for altered signals.

### 2.2 TCP Endpoints (`tcp.rs`)
- **`TcpEndpoint`:**
  - Operates in either raw TCP mode or RFC 2217 mode.
  - `connect_client()` and `bind_server()` constructors.
  - Non-blocking background read/write loops interfacing with Tokio channels.
  - Fully implements `omniport_core::AsyncEndpoint`.

### 2.3 UDP Endpoints (`udp.rs`)
- **`UdpEndpoint`:**
  - Binds to local socket and transmits datagrams directly to designated target address.
  - Implements `omniport_core::AsyncEndpoint` for packet-oriented network serial bridges.

### 2.4 Mutual TLS Endpoints (`tls.rs`)
- **`TlsEndpoint`:**
  - Uses `tokio-rustls` and `rustls` 0.23 with `ring` crypto provider.
  - Enforces mutual authentication: server verifies client certificate against root CA; client verifies server identity against root CA.
  - Eliminates plaintext vulnerability in remote serial tunnels.

---

## 3. Verification & Benchmarks

### 3.1 Unit Tests (`cargo test -p omniport-net`)
- `test_rfc854_iac_escaping`: Verified raw binary escaping and unescaping roundtrip fidelity.
- `test_rfc2217_command_encoding_and_decoding`: Verified command framing, opcode serialization, and subnegotiation parsing.
- `test_modem_pin_state_mapping`: Verified modem bitfield mapping to `PinState::CTS`, `DCD`, `DSR`, `RI`.
- `test_pin_state_to_control_commands`: Verified state transitions generate `CONTROL_DTR_ON`, `CONTROL_RTS_ON`, and break signals.
- `test_tcp_raw_loopback_roundtrip`: Verified TCP loopback client/server serial event transfer.
- `test_tcp_rfc2217_modem_pin_sync`: Verified end-to-end RFC 2217 `NOTIFY_MODEMSTATE` pin change event propagation over live TCP.
- `test_udp_bidirectional_loopback`: Verified UDP datagram send and receive loopback.
- `test_mtls_loopback_handshake_and_io`: Generated test CA, server, and client X.509 certs via `rcgen` and verified live mTLS handshake and bidirectional payload encryption.

### 3.2 Property-Based Tests (`tests/net_proptest.rs`)
- `prop_rfc854_iac_escaping_roundtrip`: Fuzzed arbitrary binary payloads (0..1024 bytes) ensuring 100% byte equality through Telnet escaping.
- `prop_rfc2217_baudrate_command_roundtrip`: Fuzzed baud rates from 300 to 4,000,000 baud.
- `prop_rfc2217_modem_bits_roundtrip`: Fuzzed all 256 modem notification byte permutations.
- `prop_pin_control_command_generation`: Fuzzed all combinations of boolean signal assertions (RTS, DTR, BREAK).

---

## 4. Architectural Invariants Sign-off
- [x] **FR-202 (Routing Matrix):** TCP, UDP, mTLS, and RFC 2217 endpoints implement `AsyncEndpoint`.
- [x] **RFC 2217 Compliance:** Complete Telnet and RFC 2217 subnegotiation state machine with bidirectional modem pin synchronization.
- [x] **Zero-Copy Routing:** Network payloads wrapped in `bytes::Bytes`.
- [x] **mTLS Encryption:** Mutual authentication via `rustls` 0.23 with X.509 certificate validation.
