---
name: step-runner
description: "Executes the next uncompleted engineering step in docs/PROGRESS.md with full verification, testing, and state updates. Invoke when starting or advancing a step."
---

# Goal
Advance the OmniPort build by executing exactly ONE step from `docs/PROGRESS.md` with strict verification before marking it complete.

# Instructions
1. **Inspect State:** Read `docs/PROGRESS.md` and identify the current active Step (or the step requested by the user).
2. **Consult Specs:** Read the relevant requirement IDs in `docs/SRS.md` and wire layouts in `docs/ICD.md`.
3. **Formulate Implementation Plan:** Present a concise, file-by-file plan for the step (traits, structs, algorithms, and unit tests to add).
4. **Implement & Verify:**
   - Write the implementation code and unit/property tests.
   - Run `cargo test --workspace` and `cargo clippy --workspace -- -D warnings`.
   - Fix any compiler warnings or test failures before proceeding.
5. **Update Progress:** Mark the completed step `[x]` in `docs/PROGRESS.md` and summarize the verification results.

# Constraints
- Do NOT jump ahead to future steps until `cargo test --workspace` passes for the current step.
- Do NOT introduce blocking I/O inside `tokio` async tasks.
