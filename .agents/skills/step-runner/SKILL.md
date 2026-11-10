---
name: step-runner
description: "Executes the next uncompleted engineering step in docs/PROGRESS.md with full verification, testing, and state updates. Invoke when starting or advancing a step."
---

# Goal
Advance the OmniPort build by executing exactly ONE step from `docs/PROGRESS.md` with strict verification before marking it complete.

# Instructions
1. **Inspect State:** Read `docs/PROGRESS.md` and identify the current active Step (or the step requested by the user).
2. **Consult Specs:** Read the relevant requirement IDs in `docs/SRS.md`, wire layouts in `docs/ICD.md`, and accepted decisions in `docs/adr/`.
3. **Formulate Implementation Plan:** Present a concise, file-by-file plan for the step (traits, structs, algorithms, and unit tests to add).
4. **Implement & Verify:**
   - Write the implementation code and unit/property tests.
   - Locally, run only formatting (`cargo fmt --all`) and crate-scoped checks for touched crates (`cargo clippy -p <crate> --all-targets --locked -- -D warnings`, `cargo test -p <crate> --locked`); do NOT run full `--workspace` sweeps locally when GitHub Actions CI is available unless reproducing a CI failure or asked by the user.
   - Run the full workspace verification suite on GitHub Actions (`.github/workflows/ci.yml` on `push` / `pull_request`, watched via `gh run watch`) and fix any compiler warnings or test failures before marking completion.
5. **Update Progress:** Mark the completed step `[x]` in `docs/PROGRESS.md` and summarize the verification results (including GitHub Actions run status when pushed).

# Constraints
- Do NOT jump ahead to future steps until targeted local checks and the full verification gate (via GitHub Actions when pushed, or local when offline/requested) pass for the current step.
- Do NOT introduce blocking I/O inside `tokio` async tasks.
