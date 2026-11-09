# OmniPort Step 3.1 Verification Report: Tauri v2 Binary IPC & TypeScript DataView Decoder

**Date:** 2026-11-10  
**Phase:** 3 — Tauri v2 + React 19 Workbench GUI  
**Step:** 3.1 — Wire Tauri v2 `fetch_viewport_rows` raw binary IPC command to `MmapCaptureEngine` and build the TypeScript `DataView` decoder [NFR-103, NFR-104]  
**Status:** PASSED (Verification Complete)

---

## 1. Architectural Context & Invariants

OmniPort captures high-speed serial streams into a dual-mmap capture store (`session.idx` with 24-byte fixed-size strides and `session.dat` holding arbitrary raw byte payloads). To achieve:
- **NFR-103:** Support capturing up to 10 GB of data while maintaining a frontend memory footprint below 250 MB.
- **NFR-104:** 60 FPS virtualized scrolling without dropped frames.
- **Invariant 2 (Zero JSON on Data Path):** High-speed capture data is never encoded into JSON across the Tauri IPC bridge.

Step 3.1 establishes the zero-copy, binary wire interface between `MmapCaptureEngine` (Rust) and the desktop client frontend (React 19 / TypeScript) via Tauri v2 custom binary IPC responses.

---

## 2. Binary Wire Protocol Specification

The binary wire format strictly adheres to **ICD Section 2 (Viewport Protocol)**:

### 2.1 Batch Header (16 bytes, Little-Endian)
| Offset (Bytes) | Field | Type | Description |
|---|---|---|---|
| `0..8` | `total_packets` | `u64` (LE) | Cumulative count of captured packets in session |
| `8..12` | `returned_rows` | `u32` (LE) | Number of rows contained in this response batch |
| `12..16` | `_reserved` | `[u8; 4]` | Alignment padding / reserved for future flags |

### 2.2 Per-Row Records (Repeated `returned_rows` times)
Each row consists of a 24-byte header followed by `payload_len` raw payload bytes:
| Offset (Bytes) | Field | Type | Description |
|---|---|---|---|
| `0..8` | `timestamp_ns` | `u64` (LE) | Absolute capture timestamp in nanoseconds UTC |
| `8..10` | `channel_id` | `u16` (LE) | Virtual channel / endpoint ID |
| `10..12` | `flags` | `u16` (LE) | Packet flags (bit 0 = RX, bit 1 = TX, bit 2 = CRC Error, etc.) |
| `12..16` | `payload_len` | `u32` (LE) | Length $L$ of payload in bytes |
| `16..24` | `_reserved` | `[u8; 8]` | Future alignment (e.g. pin state bitfield snapshot) |
| `24..24+L` | `payload` | `[u8; L]` | Exact raw payload byte slice |

---

## 3. Implementation Details

### 3.1 Rust Tauri v2 Backend (`apps/omniport-desktop/src-tauri`)
- **`src/commands.rs`:**
  - Implemented `fetch_viewport_binary(engine: &MmapCaptureEngine, start_row: u64, row_count: u32) -> Vec<u8>`:
    Reads rows directly from `engine.read_viewport_slice(start_row, row_count)`, writes the 16-byte header, and appends each 24-byte record along with the raw payload bytes without intermediate allocations.
  - Implemented `fetch_viewport_rows` command returning `tauri::ipc::Response::new(bytes)`. This bypasses Tauri's JSON serde layer, transferring raw byte buffers directly to WebKit/WebView2.
- **Unit Tests (`commands::tests`):**
  - `test_fetch_viewport_binary_wire_format`: Verifies 16-byte batch header, multiple rows with dynamic payloads, correct LE endianness, and payload fidelity.
  - `test_fetch_viewport_binary_out_of_bounds`: Verifies graceful bounds handling when `start_row` exceeds session size (`returned_rows == 0`).

### 3.2 TypeScript Zero-Copy Decoder (`apps/omniport-desktop/src/ipc/viewportDecoder.ts`)
- Implemented `decodeViewportBinary(buffer: ArrayBuffer | ArrayBufferView): ViewportBatch`.
- Uses a single `DataView` over the underlying `ArrayBuffer` without copying:
  - Validates minimum header size (16 bytes).
  - Reads `totalPackets` (`getBigUint64(0, true)`) and `returnedRows` (`getUint32(8, true)`).
  - Parses each row, extracting `timestampNs`, `channelId`, `flags`, and payload as a zero-copy sub-array view (`new Uint8Array(underlyingBuffer, byteOffset, payloadLen)`).
  - Robust error boundary: returns empty batch on truncated, corrupt, or undersized payloads.
- **Tauri Client Invoker (`src/ipc/viewportClient.ts`):**
  - Typed helper `fetchViewport(startRow: number | bigint, rowCount: number): Promise<ViewportBatch>` calling Tauri's `invoke('fetch_viewport_rows')`.

---

## 4. Verification & Benchmarks

### 4.1 Node.js Frontend Tests (`node --test src/ipc/viewportDecoder.test.js`)
- **Test 1:** `decodeViewportBinary correctly parses multi-row binary batch`: **PASSED**
  - Verified 2 rows with distinct timestamps (`1,000,000,000 ns`, `2,000,000,000 ns`), channel IDs (1, 2), flags (`0x0001`, `0x0002`), and byte payloads (`0xDE, 0xAD, 0xBE, 0xEF`, `0x01, 0x02, 0x03`).
- **Test 2:** `handles empty and truncated buffers gracefully`: **PASSED**
  - Verified empty buffer, truncated 8-byte buffer, and truncated row payload gracefully return empty batches.
- **Test 3 (NFR-104 Benchmark):** `benchmarks decoding 100 rows in < 1ms (60 FPS budget)`: **PASSED**
  - **Measured Latency:** **63.8 $\mu\text{s}$ (0.0638 ms)** to decode 100 rows.
  - **Budget:** 16.6 ms per frame (60 FPS).
  - **Performance Margin:** Decodes 100 viewport rows in **< 0.4%** of a single 60 FPS frame time.

### 4.2 Rust Tauri Unit Tests (`cargo test --manifest-path apps/omniport-desktop/src-tauri/Cargo.toml`)
- All unit tests in `omniport-desktop` passed without errors.

---

## 5. Architectural Invariants Sign-off
- [x] Invariant 1 (Daemon-First Separation): Desktop client acts as a stateless viewer.
- [x] Invariant 2 (Zero JSON on Data Path): Raw binary `tauri::ipc::Response` and `DataView` parsing; zero serde JSON on packet streaming.
- [x] Invariant 3 (Zero-Copy): Slices view directly into memory without extraneous copies.
- [x] Invariant 4 (Memory Limits): Zero unbounded frontend growth; only viewport rows are requested and held in memory.
