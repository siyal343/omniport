# omniport/docs

Everything a contributor (human or agent) needs to know *what* OmniPort must do and
*why* it is built this way.

## Start here

- **[PROGRESS.md](PROGRESS.md)** — the single implementation tracker: phases, steps,
  verification protocol. Check this before starting work; use the `/step-runner`
  skill to execute one step at a time.
- **[AUDIT_2026-11-10.md](AUDIT_2026-11-10.md)** — audit snapshot: every defect found
  and fixed, the architecture review outcome, GUI test evidence, verification
  results, and the documented known gaps.
- **[AGENTS.md](../AGENTS.md)** — repository operating manual: architectural
  invariants, verification policy, workspace commands and the step protocol.

## Engineering specifications (normative)

| Document | Contents |
| :--- | :--- |
| [SRS.md](SRS.md) | Functional requirements (FR-*) and non-functional requirements (NFR-*) |
| [ICD.md](ICD.md) | Interface control: dual-mmap index layout, pin/flag bit tables, Tauri viewport wire format, daemon control-plane transport |
| [SCENARIO.md](SCENARIO.md) | Declarative CI scenario contract: schema, supported endpoint types, validation rules, pin assertions and report format |
| [VV_RTM.md](VV_RTM.md) | Verification plan and requirements traceability matrix |
| [RELEASE.md](RELEASE.md) | Windows installer build/packaging checklist, packaged-app smoke test and signing follow-ups |
| [adr/](adr/) | Architecture decision records, one decision per file |

Specs are versioned: change them in the same commit as the code they constrain.
Requirement IDs (`FR-201`, `NFR-104`, …) are cited from source comments and tests;
keep those references accurate when requirements move.

## Architecture Decision Records

`docs/adr/` holds one decision per file (`NNNN-kebab-title.md`). ADRs are immutable
once accepted — to change a decision, add a new ADR that supersedes the old one
instead of editing history. ADR-0007 (single-owner viewport contract) is the first
decision with a frozen cross-language fixture; the rule of thumb is that every
"surprising without context, hard to reverse, real trade-off" decision gets a file.

## Conventions for this directory

- Keep `PROGRESS.md` as the only tracker; do not add a second progress file.
- The test suite is the executable part of these specs: `cargo test --workspace`,
  the desktop `npm test` (including the ICD §2 golden-fixture contract), and the
  Tauri host tests.
- GUI evidence lives outside version control in `gui-test-screenshots/` (gitignored).
