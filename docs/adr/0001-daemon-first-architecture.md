# ADR-0001: Daemon-first architecture

- **Status:** Accepted

## Decision

The core engine runs as a headless `omniportd` daemon; the GUI (`omniport-desktop`)
and CLI (`omniport-cli`) connect over gRPC (Unix Domain Socket / Named Pipe) as
stateless clients, so closing a client never drops active virtual serial ports.

## Implementation status

As of the 2026-11-10 audit the control plane is a 4-byte length-prefixed JSON codec
over TCP, and the desktop host still owns an in-process capture store rather than
being a daemon client. See `docs/AUDIT_2026-11-10.md` §8 and Phase 6 steps 6.4/6.5
in `docs/PROGRESS.md`.
