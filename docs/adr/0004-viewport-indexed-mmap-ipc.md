# ADR-0004: Viewport-indexed mmap IPC

- **Status:** Accepted

## Decision

Keep capture data in dual memory-mapped files (`session.idx` + `session.dat`) and
transfer only visible viewport rows over Tauri v2 raw binary IPC.

The wire format is specified in `docs/ICD.md` §1–§2 and frozen by
`fixtures/viewport_v1.json` (see ADR-0007).
