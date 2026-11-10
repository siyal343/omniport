# ADR-0007: Single-owner viewport wire contract

- **Status:** Accepted
- **Date:** 2026-11-10

## Context

The ICD §2 viewport batch layout (the binary payload of the Tauri
`fetch_viewport_rows` command) was transcribed by hand in five places: the capture
writer, the extcap re-parser, the Tauri command test, the TypeScript decoder, and
the TypeScript decoder test. Nothing tied them together, so a header or stride
change could compile and pass every Rust test while silently corrupting the GUI.

## Decision

ICD §2's viewport batch layout is owned by exactly one module,
`omniport_capture::viewport::ViewportBatch`, and frozen by an independent,
checked-in fixture (`fixtures/viewport_v1.json`) that both the Rust encoder and the
TypeScript decoder must reproduce.

The fixture is generated from the ICD text alone
(`fixtures/generate_viewport_fixture.mjs`), not from either implementation, so drift
in either language fails a test instead of a display. The TypeScript decoder is
asserted against the same bytes by
`apps/omniport-desktop/src/ipc/viewportDecoder.contract.test.js`.

## Consequences

- Payloads are copied into the wire `Vec`; ADR-0003's zero-copy claim holds for
  routing, not for viewport serialization.
- Reserved wire bits must stay zero, because typed `PinState`/`FrameFlags` values do
  not round-trip undefined bits.
- Any wire-format change now requires regenerating the fixture from an updated ICD,
  and both languages fail loudly until they agree.
