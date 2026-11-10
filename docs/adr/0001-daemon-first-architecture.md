# ADR-0001: Daemon-first architecture

- **Status:** Superseded by [ADR-0008](0008-json-tcp-daemon-control-plane.md)

## Decision

The core engine runs as a headless `omniportd` daemon; the GUI (`omniport-desktop`)
and CLI (`omniport-cli`) connect over gRPC (Unix Domain Socket / Named Pipe) as
stateless clients, so closing a client never drops active virtual serial ports.

## Implementation status

Superseded by [ADR-0008](0008-json-tcp-daemon-control-plane.md) (Phase 6 steps
6.4, 6.5, and 6.8): daemon-first separation stands, `omniport-desktop` connects
through the `CaptureStore` seam (`DaemonCaptureStore` default in release builds),
the legacy `.proto` drafts are deleted, and the shipped control plane is 4-byte
big-endian length-prefixed JSON over TCP (`docs/ICD.md` §3). Unix Domain Socket
/ Named Pipe transport remains a future local-IPC option.
