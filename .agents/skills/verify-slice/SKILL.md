---
name: verify-slice
description: "Runs cargo check, clippy, unit tests, and checks binary struct layout invariants against docs/ICD.md and docs/VV_RTM.md."
---

# Goal
Validate that the workspace compiles cleanly, passes all tests, and adheres to the Requirements Traceability Matrix (`docs/VV_RTM.md`).

# Instructions
1. Locally, verify formatting (`cargo fmt --all -- --check`) and targeted crate-scoped checks for touched crates (`cargo clippy -p <crate> --all-targets --locked -- -D warnings`, `cargo test -p <crate> --locked`). Do not run full `--workspace` clippy or test suites locally when GitHub Actions CI is available, unless reproducing a failing CI job or explicitly requested.
2. Run the full workspace verification suite (`cargo fmt --all -- --check`, `cargo clippy --workspace`, `cargo test --workspace`, plus `apps/omniport-desktop` and Tauri host suites) on GitHub Actions (`.github/workflows/ci.yml` triggered on `push` / `pull_request`, monitored with `gh run watch`).
3. Report test coverage mapping (scoped local checks and GitHub Actions run status/URL) back to the Requirement IDs in `docs/VV_RTM.md`.
