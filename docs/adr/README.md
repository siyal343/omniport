# Architecture Decision Records

One decision per file, numbered `NNNN-kebab-title.md`. ADRs are immutable once
accepted: add a new ADR that supersedes an old one rather than editing history.

| ADR | Title | Status |
| :--- | :--- | :--- |
| [0001](0001-daemon-first-architecture.md) | Daemon-first architecture | Superseded by [ADR-0008](0008-json-tcp-daemon-control-plane.md) |
| [0002](0002-hybrid-windows-driver-strategy.md) | Hybrid Windows driver strategy | Accepted |
| [0003](0003-zero-copy-pipeline.md) | Zero-copy pipeline | Superseded by [ADR-0009](0009-scoped-zero-copy-pipeline.md) |
| [0004](0004-viewport-indexed-mmap-ipc.md) | Viewport-indexed mmap IPC | Accepted |
| [0005](0005-runtime-c-struct-dissector-and-lua.md) | Runtime C-struct dissector and Lua 5.4 simulation | Accepted |
| [0006](0006-apache-2-0-monorepo-license.md) | Apache-2.0 monorepo license | Accepted |
| [0007](0007-single-owner-viewport-wire-contract.md) | Single-owner viewport wire contract | Accepted |
| [0008](0008-json-tcp-daemon-control-plane.md) | Length-prefixed JSON over TCP control plane | Accepted |
| [0009](0009-scoped-zero-copy-pipeline.md) | Scoped zero-copy pipeline | Accepted |

The two decisions flagged by the 2026-11-10 audit — ADR-0001 (transport and
daemon-client status) and ADR-0003 (zero-copy scope) — are resolved by
[ADR-0008](0008-json-tcp-daemon-control-plane.md) and
[ADR-0009](0009-scoped-zero-copy-pipeline.md) after Phase 6 steps 6.4, 6.5, and
6.8. See [`../AUDIT_2026-11-10.md`](../AUDIT_2026-11-10.md) and
[`../PROGRESS.md`](../PROGRESS.md).
