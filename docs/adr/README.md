# Architecture Decision Records

One decision per file, numbered `NNNN-kebab-title.md`. ADRs are immutable once
accepted: add a new ADR that supersedes an old one rather than editing history.

| ADR | Title | Status |
| :--- | :--- | :--- |
| [0001](0001-daemon-first-architecture.md) | Daemon-first architecture | Accepted |
| [0002](0002-hybrid-windows-driver-strategy.md) | Hybrid Windows driver strategy | Accepted |
| [0003](0003-zero-copy-pipeline.md) | Zero-copy pipeline | Accepted |
| [0004](0004-viewport-indexed-mmap-ipc.md) | Viewport-indexed mmap IPC | Accepted |
| [0005](0005-runtime-c-struct-dissector-and-lua.md) | Runtime C-struct dissector and Lua 5.4 simulation | Accepted |
| [0006](0006-apache-2-0-monorepo-license.md) | Apache-2.0 monorepo license | Accepted |
| [0007](0007-single-owner-viewport-wire-contract.md) | Single-owner viewport wire contract | Accepted |

Decisions currently flagged for follow-up by the 2026-11-10 audit: ADR-0001
(transport/GUI client status) and ADR-0003 (zero-copy scope). See
[`../AUDIT_2026-11-10.md`](../AUDIT_2026-11-10.md) and Phase 6 of
[`../PROGRESS.md`](../PROGRESS.md).
