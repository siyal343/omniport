# ADR-0009: Scoped zero-copy pipeline (supersedes ADR-0003)

- **Status:** Accepted
- **Date:** 2026-11-10
- **Supersedes:** [ADR-0003](0003-zero-copy-pipeline.md)

## Context

[ADR-0003](0003-zero-copy-pipeline.md) stated broadly that reference-counted
`bytes::Bytes` slices inside `SerialEvent` prevent heap copies during 1:N
splitting and capture tapping.

The 2026-11-10 audit (`docs/AUDIT_2026-11-10.md`) and
[ADR-0007](0007-single-owner-viewport-wire-contract.md) verified that while
`bytes::Bytes` eliminates payload heap copies on routing fan-out and tap
dispatch, other stages of the pipeline necessarily copy payload bytes — notably
copy-on-write fault corruption, persistence into the `session.dat` memory map,
ICD §2 `ViewportBatch` wire assembly, and control-plane JSON/base64 framing.

## Decision

Scope the zero-copy guarantee explicitly to **routing, 1:N splitting,
non-mutating fault pass-through, and capture-tap dispatch**, and treat every
serialization or mutation boundary as an explicit, bounded copy:

1. **Zero-copy guaranteed on routing and dispatch:**
   - `SerialEvent.payload` (`crates/omniport-core/src/lib.rs`) holds a
     reference-counted `bytes::Bytes`.
   - `RoutingGraph::route` (`crates/omniport-core/src/routing.rs`) fans frames
     across 1:N edges into each destination `BoundedEventQueue` by cloning
     `SerialEvent` (atomic refcount increment; zero payload heap copy).
   - `FaultInjector::inject` (`crates/omniport-core/src/fault.rs`) preserves
     `event.payload` zero-copy when only timing delays or rate limits apply,
     allocating `BytesMut` via `extend_from_slice` only if a corruption fault
     (`corrupt_nth_crc`, `bit_error_rate`, or `gilbert_elliott`) mutates bytes.
   - `StreamPacketizer` (`crates/omniport-core/src/framing.rs`) slices
     `Delimiter`, `IdleGap`, and `LengthField` frames zero-copy out of `rx_buf`
     via `split_to` / `split` + `freeze()`, and strips CRCs via `Bytes::slice`
     (`Raw` copies at ingress; `Cobs` and `Slip` allocate decoded payloads).
   - `DaemonEngine::record_event` and `MmapCaptureEngine::append_event` accept
     `&SerialEvent` without cloning `Bytes` on the tapping caller; on the
     consumer side, `ViewportBatch::decode` (`crates/omniport-capture/src/viewport.rs`)
     borrows `&'a [u8]` payload slices zero-copy from the encoded batch buffer.
2. **Explicit copy boundaries (outside the zero-copy claim):**
   - **Capture persistence (`MmapCaptureEngine::append_frame`):** copies
     `&event.payload` once into `session.dat` (`dat_mmap`).
   - **ICD §2 viewport serialization (`ViewportBatch::encode` / `encode_to_vec`):**
     copies each visible row's 24-byte `FrameIndexEntry` header and payload
     slice from `dat_mmap` into a contiguous `Vec<u8>` wire buffer (bounded by
     `MAX_VIEWPORT_ROWS = 4096`, per ADR-0007).
   - **Control-plane viewport transport (`omniport-proto` `Response::ViewportBatch`):**
     base64-encodes the `Vec<u8>` batch inside a length-prefixed JSON frame
     over TCP (bounded by `MAX_VIEWPORT_BATCH_BYTES = 8 MiB` raw, `docs/ICD.md` §3),
     and `DaemonCaptureStore` decodes back into `Vec<u8>` before returning
     `tauri::ipc::Response` to the WebView (`fetch_viewport_rows`, ADR-0004).

## Consequences

- Hot-path routing and 1:N splitter fan-out remain allocation-free for
  unmutated frames.
- Engineering claims match the shipped code: viewport serialization and
  control-plane transport are explicitly copy-bounded (`MAX_VIEWPORT_ROWS`,
  `MAX_VIEWPORT_BATCH_BYTES`) rather than zero-copy.
- Any future zero-copy optimization on viewport delivery (such as direct local
  shared-memory mapping between `omniportd` and `omniport-desktop`) would be an
  additive `CaptureStore` adapter under ADR-0008 without changing the ICD §2
  wire contract or core routing invariants.
