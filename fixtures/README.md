# Viewport wire-format contract fixture

`viewport_v1.json` is the frozen, independent oracle for the ICD §2 viewport
batch format (`docs/ICD.md` §2) — the binary payload of the Tauri
`fetch_viewport_rows` command.

It exists because the same layout used to be transcribed by hand in five places:
the capture writer (`omniport-capture::MmapCaptureEngine::read_viewport`), the
Wireshark extcap re-parser, the Tauri command test, the TypeScript decoder, and
the TypeScript decoder test. Nothing tied them together, so a header or stride
change could compile, pass every Rust test, and silently corrupt the GUI.

## Provenance

`generate_viewport_fixture.mjs` is written from the ICD text alone. It does not
import, call or mirror any Rust or TypeScript implementation, and it is the only
thing that may regenerate the JSON:

```sh
node fixtures/generate_viewport_fixture.mjs
```

`fixtures/viewport_v1.json` is marked `"frozen": true`. Do not hand-edit it.

## Consumers

| Language | Test | Asserts |
| :--- | :--- | :--- |
| Rust | `crates/omniport-capture/tests/viewport_contract.rs` | the encoder reproduces `wire_hex` byte-for-byte, every row field crosses the seam, and the decoder reads it back |
| TypeScript | `apps/omniport-desktop/src/ipc/viewportDecoder.contract.test.js` | `decodeViewportBuffer` reproduces the expected rows from the same `wire_hex` |

If either implementation drifts from the ICD, one of those tests fails. The
single owner of the layout is `omniport_capture::viewport::ViewportBatch`
(ADR-007).

## Encoding notes

- Little-endian throughout.
- u64 values are decimal **strings** in the JSON so JavaScript cannot lose
  precision above 2^53.
- `pins` uses only bits 0–6 (RTS, CTS, DTR, DSR, DCD, RI, BREAK); bit 7 is
  reserved and must be zero, matching the typed `PinState` that crosses the seam.
