# ADR-0010: Ephemeral capture sessions (no persistence across daemon restarts)

- **Status:** Accepted
- **Date:** 2026-11-11

## Context

`MmapCaptureEngine::create` truncates and re-sizes `session.idx` and
`session.dat` on every daemon start (`crates/omniport-capture/src/lib.rs`). The
GUI reads only the current session through ICD §2 viewport batches, and the
capture store is a fixed-capacity, ring-less writable mapping (frames are
dropped once the index or data region is exhausted).

Step 7.8 asked for an explicit decision: implement reopen/resume of an existing
session, or record that capture sessions are ephemeral. Stale captured data
being silently re-read after a restart would be worse than a clean empty
session: the frontend, topology, and device assignment may have changed, and
there is no session identifier or validation header on disk to detect a
mismatch.

## Decision

Capture sessions are **ephemeral per daemon process**:

- `MmapCaptureEngine::create` remains truncating: every `omniportd` start begins
  a new, zero-length session.
- No reopen/resume, rotation, crash recovery, or session-identity validation is
  implemented for v0.1.
- The files persist physically (they are memory-mapped files) but their
  *content* is not a supported resume input; nothing in the product reads a
  previous run's frames.
- Durable output is provided by explicit export paths: PCAPNG export
  (`omniport-capture` / CLI), Wireshark extcap streaming, and the GUI's capture
  session exports.

Any future persistence feature must add a versioned session header with
geometry/session identity, validate it on open, and be recorded in a new ADR
that supersedes this one.

## Consequences

- Daemon restart intentionally discards the capture; the GUI shows an empty
  session with `Total: 0 frames`, which is the expected, testable behaviour.
- Users who need history must export before stopping the daemon; this is
  documented in the CLI help / release docs.
- The capture store stays a single-writer, fixed-capacity, O(1)-seek structure
  with no recovery logic or torn-write handling.
- `docs/ICD.md` §1.3 and `docs/SRS.md` FR-401 record the non-goal so it is not
  silently assumed by clients or HIL test plans.
