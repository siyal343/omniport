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
