# README project homepage

User approved rewriting the README as a concise Chinese project homepage and
requested implementation, commit/push to main, and CI verification.

## Requirements

- Explain ESOP's current real-time control runtime and Linux software-master
  status, preserving its full name and an accurate hardware qualification boundary.
- Provide a main-branch CI badge, core capability table, practical prerequisites,
  simulation/help/replay commands with consistent parameters, architecture diagram,
  grouped workspace navigation, quality commands, evidence and documentation links.
- Keep the CLI read-only and the real-time/supervision boundary accurate.
- Link detailed register/eBPF/history material in existing documentation.
- Keep code/interfaces unchanged and preserve user-owned paseo.json.

## Acceptance

- [x] Relative links and crate/document paths exist; badge targets the main workflow.
- [x] Commands and parameters match the current CLI and Makefile.
- [x] Simulation/replay examples use the same scenario, mode, cycles and paths.
- [x] Current capabilities and evidence are distinguished from deferred hardware work.
- [x] Diff checks pass; commits are pushed to main and CI is checked.

This is a documentation-only task. No new tests or protocol/spec changes are
required; existing source/contracts provide the authority for the wording.

## Validation

Checked 44 links/anchors and seven Make targets, balanced code fences, matched
simulation/replay options and capture paths, and executed the actual --help path.
The 10,000-cycle release simulation and exact replay passed in isolated
build/readme-validation output directories, with identical scenario/state/event
summaries and zero activated heap operations. Reviewed the Mermaid health input
against the eBPF/MLG contract and kept the CLI read-only. Spec-sync review found
no new API or implementation contract to record.

Work commit 968008d is pushed to main. Its CI passed all 12 jobs:
https://github.com/ra1mj/ESOP/actions/runs/41650938043
The GitHub README blob matches the local file exactly.
