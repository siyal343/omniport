# ADR-0003: Zero-copy pipeline

- **Status:** Accepted

## Decision

Reference-counted `bytes::Bytes` slices inside `SerialEvent` prevent heap copies
during 1:N splitting and capture tapping.

## Scope note

The 2026-11-10 audit found the guarantee is correctly held on the routing and
capture-tapping paths, but not on viewport serialization: the ICD §2 batch copies
each payload into the wire buffer (see ADR-0007). The decision text above should be
scoped explicitly to routing/tapping; tracked as Phase 6 step 6.8 in
`docs/PROGRESS.md`.
