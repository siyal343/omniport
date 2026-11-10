# ADR-0003: Zero-copy pipeline

- **Status:** Superseded by [ADR-0009](0009-scoped-zero-copy-pipeline.md)

## Decision

Reference-counted `bytes::Bytes` slices inside `SerialEvent` prevent heap copies
during 1:N splitting and capture tapping.

## Scope note

Superseded by [ADR-0009](0009-scoped-zero-copy-pipeline.md) (Phase 6 step 6.8):
`bytes::Bytes` zero-copy holds on routing, 1:N splitting, non-mutating fault
pass-through, and capture-tap dispatch, while fault corruption, dual-mmap
persistence, ICD §2 `ViewportBatch` serialization (ADR-0007), and control-plane
base64/JSON framing copy payload bytes.
