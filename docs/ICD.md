# Interface Control Document (ICD)

## 1. Dual-Mmap Index Entry (`session.idx` — 24 Bytes Little-Endian)
| Offset | Size | Type | Field |
| :--- | :--- | :--- | :--- |
| `0x00` | 8 B | `u64` | `timestamp_ns` |
| `0x08` | 8 B | `u64` | `data_offset` (offset inside `session.dat`) |
| `0x10` | 4 B | `u32` | `payload_len` |
| `0x14` | 2 B | `u16` | `endpoint_id` |
| `0x16` | 1 B | `u8`  | `flags` (`FrameFlags` bitmask, table below) |
| `0x17` | 1 B | `u8`  | `pins` (`PinState` bitmask, table below) |

### 1.1 `pins` — PinState bit assignments (normative)

| Bit | Mask | Signal |
| :--- | :--- | :--- |
| 0 | `0x01` | RTS |
| 1 | `0x02` | CTS |
| 2 | `0x04` | DTR |
| 3 | `0x08` | DSR |
| 4 | `0x10` | DCD |
| 5 | `0x20` | RI |
| 6 | `0x40` | BREAK |
| 7 | `0x80` | reserved (must be 0) |

### 1.2 `flags` — FrameFlags bit assignments (normative)

| Bit | Mask | Meaning |
| :--- | :--- | :--- |
| 0 | `0x01` | `DIR_TX` (1 = TX, 0 = RX) |
| 1 | `0x02` | `ERR_PARITY` |
| 2 | `0x04` | `ERR_FRAMING` |
| 3 | `0x08` | `ERR_OVERRUN` |
| 4 | `0x10` | `ERR_CRC` |
| 5 | `0x20` | `FAULT_INJECT` |
| 6 | `0x40` | `PIN_CHANGE` |
| 7 | `0x80` | reserved (must be 0) |

### 1.3 Session lifetime (ephemeral, ADR-0010)

`MmapCaptureEngine::create` truncates `session.idx` / `session.dat` at daemon start:
capture sessions are **ephemeral per daemon process**. A restart begins a new,
zero-length session (the GUI reports `Total: 0 frames`); previous file content is never
resumed, validated, or re-read. Durable history is provided by PCAPNG export
(`omniport-capture` / `omniport-cli`) and Wireshark extcap streaming. Persistence,
rotation, and crash recovery are out of scope for v0.1 — a future implementation must
add a versioned session header and supersede
[ADR-0010](adr/0010-ephemeral-capture-sessions.md).

## 2. Tauri Binary Viewport Wire Format (`fetch_viewport_rows`)
- Header (16 bytes): `[total_packets: u64 (8B)][returned_rows: u32 (4B)][reserved: u32 (4B)]`
- Repeated `returned_rows` times: `[FrameIndexEntry (24B)][payload_bytes (N bytes)]`
- All integers are little-endian. The `reserved` word MUST be zero.
- `total_packets` is the session row count (not clamped by the request); `returned_rows`
  is the number of rows actually present after clamping `start_row + row_count` to the
  session length. An out-of-range `start_row` yields zero rows and still reports the total.
- `data_offset` inside each row is the byte offset of the payload in `session.dat`; the
  payload bytes follow the 24-byte entry so a decoder can walk the batch without the file.
- Normative test vectors: `fixtures/viewport_v1.json` (generated from this document by
  `fixtures/generate_viewport_fixture.mjs`). Both the Rust encoder
  (`crates/omniport-capture/tests/viewport_contract.rs`) and the TypeScript decoder
  (`apps/omniport-desktop/src/ipc/viewportDecoder.contract.test.js`) MUST reproduce it.

## 3. Daemon Control-Plane Wire Format (`omniportd` ↔ Clients)

The control plane between `omniportd` and stateless clients (`omniport-cli`,
`omniport-desktop`) is defined authoritatively in `crates/omniport-proto/src/types.rs`
and `crates/omniport-proto/src/codec.rs` (the legacy `.proto` drafts were deleted to keep
one schema owner).

### 3.1 Transport and Framing
- **Transport:** TCP stream (`127.0.0.1:50051` by default, configurable via `omniportd --bind`).
- **Frame layout:** `[len: u32 big-endian][payload: UTF-8 JSON]`.
- **Frame limit:** `MAX_FRAME_SIZE = 16 MiB` (`16 * 1024 * 1024` bytes), checked before
  allocating the read buffer and before writing (`ProtoError::FrameTooLarge`). A frame
  over the limit closes the connection.
- **Connection policy:** see §3.4 (loopback default, remote-bind opt-in, timeouts,
  connection cap and rate limit).

### 3.2 Canonical Messages (`Request` / `Response`)
Requests and responses serialize with serde's externally-tagged enum JSON representation
(unit variants as JSON strings, struct variants as single-key objects).

| `Request` variant | Purpose | Primary `Response` variant |
| :--- | :--- | :--- |
| `"HealthCheck"` | Query status, uptime, and counts | `Health { status, uptime_secs, active_endpoints, active_routes }` |
| `"ListEndpoints"` | Enumerate registered endpoints | `Endpoints(Vec<EndpointInfo>)` |
| `"GetTopology"` | Fetch endpoints + canonical routes | `Topology { endpoints, routes }` |
| `CreateVirtualPair { port_a, port_b, config }` | Provision virtual endpoint pair + default bidirectional route | `VirtualPairCreated { pair_id, port_a, port_b, success, message }` |
| `RemoveEndpoint { id }` | Unregister endpoint and prune referencing routes | `Success { message }` |
| `AddRoute { source_id, target_ids, bidirectional, fault? }` | Validate endpoints and install canonical `Route` | `RouteAdded { route_id, success, message }` |
| `RemoveRoute { route_id }` | Delete route and reconcile `RoutingGraph` edges | `Success { message }` |
| `ReadViewport { start_row, row_count }` | Fetch ICD §2 viewport batch | `ViewportBatch { bytes }` |
| `"CaptureStatus"` | Query total and dropped frame counts | `CaptureStatus { total_packets, dropped_frames }` |
| `"Shutdown"` | Initiate graceful daemon shutdown | `Success { message }` |

- `fault` on `AddRoute` and `Route` is optional (`#[serde(default, skip_serializing_if = "Option::is_none")]`)
  for wire compatibility with pre-fault callers.
- `Response::ViewportBatch.bytes` travels as a standard RFC 4648 base64 string rather
  than a JSON byte array, capped at `MAX_VIEWPORT_BATCH_BYTES = 8 MiB` raw (~10.7 MiB
  base64, inside `MAX_FRAME_SIZE`). Requests exceeding the cap return `Error { code: 413, ... }`.

### 3.3 Topology DTOs and Error Codes
- **`EndpointInfo`:** `{ id: u16, name: String, port_type: String, baud_rate: u32, is_active: bool }`.
- **`Route` (owned by `omniport_core::topology`):**
  `{ route_id: u32, source_id: u16, target_ids: Vec<u16>, bidirectional: bool, fault?: FaultConfig }`.
- **`Error { code: u32, message: String }` status codes:**
  - `400` — validation error (empty endpoint name, empty `target_ids`, self-loop, duplicate target, invalid fault probability).
  - `404` — unknown endpoint ID or route ID.
  - `409` — endpoint ID conflict or conflicting non-equal `FaultConfig` on the same directed edge.
  - `413` — ICD §2 viewport batch exceeds `MAX_VIEWPORT_BATCH_BYTES`.
  - `429` — per-connection request rate limit exceeded (`--max-requests-per-sec`); connection is then closed.
  - `503` — endpoint/pair/route ID space exhausted, `omniportd` is shutting down, the request exceeded `--request-timeout-secs`, or a new connection was rejected by `--max-connections`.
  - `500` — internal routing invariant failure.

### 3.4 Connection Policy, Limits, and Bind Security

The control plane has **no authentication**: TCP loopback relies on the OS user
boundary, and there are no per-connection peer-credential checks yet. The server
therefore applies a bounded, opt-in policy (`crates/omniportd/src/server.rs`,
CLI in `crates/omniportd/src/main.rs`):

| Setting | Default | Behaviour |
| :--- | :--- | :--- |
| bind address | `127.0.0.1:50051` | Loopback only. Any non-loopback `--bind` is refused at startup unless `--allow-remote` is passed; remote control should only be enabled behind an authenticated tunnel/network. |
| `--max-connections` | `64` | Concurrency is bounded by a semaphore. Over-limit connections are read for at most `min(request_timeout, 2 s)` and answered with `Error { code: 503, "connection limit reached" }` before closing; connections that send nothing are dropped silently. |
| `--idle-timeout-secs` | `30` | A connected client that sends no request is closed **silently** after the deadline. The transport is strictly request/response, so no unsolicited `408` frame is emitted (it would be mis-read as the response to the next request); clients reconnect on their next call. |
| `--request-timeout-secs` | `10` | If handling one request exceeds the deadline, the client receives `Error { code: 503, "request timed out" }`. |
| `--max-requests-per-sec` | `200` per connection | Sliding one-second window; exceeding it returns `Error { code: 429 }` and closes the connection. `0` disables the limit. |

Idle connections, oversized frames, and overload rejections are logged to stderr.
The desktop client adapters (`DaemonCaptureStore`, `DaemonTopology`) transparently
reconnect after an idle/daemon-restart close.
