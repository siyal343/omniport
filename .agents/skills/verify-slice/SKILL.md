---
name: verify-slice
description: "Runs cargo check, clippy, unit tests, and checks binary struct layout invariants against internal_docs/ICD.md and internal_docs/VV_RTM.md."
---

# Goal
Validate that the workspace compiles cleanly, passes all tests, and adheres to the Requirements Traceability Matrix (`internal_docs/VV_RTM.md`).

# Instructions
1. Run `cargo fmt --all -- --check` (or `cargo fmt --all` if formatting is needed).
2. Run `cargo clippy --workspace -- -D warnings`.
3. Run `cargo test --workspace -- --nocapture`.
4. Report test coverage mapping back to the Requirement IDs in `internal_docs/VV_RTM.md`.
