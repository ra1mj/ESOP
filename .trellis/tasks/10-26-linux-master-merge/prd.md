# Linux master hardening and main integration

User requested continued improvement followed by merging into main on 2026-10-26.
This explicitly continues the approved Linux R0-R2 software scope and authorizes
implementation, commits, GitHub pushes and main integration.

## Requirements

- Review the completed wire-only master at its remaining cross-layer failure
  boundaries: mailbox scheduling, runtime monitoring, capture/replay and evidence.
- Fix confirmed, reproducible software defects and add meaningful regressions.
- Preserve bounded activated runtime, complete-only publication, explicit recovery
  and the existing hardware qualification boundary.
- Merge the verified feature branch into up-to-date main and push to GitHub.
- Leave unrelated user-owned paseo.json untouched.

## Acceptance

- [x] Independent focused review findings resolved or disproved with evidence.
- [ ] Relevant regression tests and Linux make ci pass on the final code.
- [x] Specifications and public usage/limitations match the final implementation.
- [ ] GitHub main contains the completed work without a force push.

MCU ports, real hardware qualification and optional mailbox protocols remain
outside this software integration follow-up.
