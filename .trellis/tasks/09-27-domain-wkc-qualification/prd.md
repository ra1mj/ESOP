# Per-Domain cyclic WKC qualification

## Goal

Complete the deterministic software boundary for cyclic EtherCAT working-counter
(WKC) qualification by attributing explicit WKC mismatches to the owning Domain,
preserving the observed WKC, and exposing a bounded consecutive-mismatch counter
through ProcBuf and lifecycle diagnostics.

This task does not claim physical-slave, target-port, interoperability, or HIL
qualification. Those remain separate release evidence.

## Requirements

### R1 - Exact mismatch observation

- The cyclic engine must notify receive consumers only when a datagram has passed
  deadline, generation, address, size, and command validation and failed only its
  WKC comparison.
- The observation must include enough typed evidence to identify the receive slot,
  generation, expected WKC, actual WKC, and validated datagram header.
- Existing receive consumers must remain source-compatible through a default no-op
  observer.

### R2 - Deterministic Domain attribution

- A mismatch may mutate only the active Domain that owns the datagram index for the
  current receive generation.
- Scheduled multi-Domain operation must use the existing fixed-size index ownership
  table; it must not scan dynamically, allocate, or attribute control/foreign indices.
- A Domain may record at most one mismatch episode per due cycle even when it owns
  multiple cyclic datagrams.

### R3 - Domain quality semantics

- `DomainQuality` must expose `consecutive_wkc_mismatches` as a saturating `u16`.
- A due cycle containing at least one explicit WKC mismatch increments the counter
  once and must not commit staged process data.
- A due cycle without an explicit WKC mismatch resets the counter to zero, including
  successful cycles and cycles that fail for another reason such as missing, corrupt,
  late, or timed-out input.
- A non-due cycle must preserve the prior counter because no new Domain sample was
  expected.
- The Domain must retain the observed actual WKC for diagnostic comparison even though
  the payload is rejected.

### R4 - ProcBuf versioning

- The ProcBuf per-Domain quality record must expose the new counter by reusing the
  existing two-byte reserved field.
- The semantic change must bump ProcBuf ABI version 6 to version 7 and therefore
  produce a new layout hash, while preserving the structure size and field offsets.
- Version 6 attachments must be rejected rather than interpreted as if they supplied
  the new diagnostic.
- Lifecycle projection must copy the core Domain counter without weakening any current
  validity or safety gate.

### R5 - Lifecycle safety behavior

- Current-cycle WKC failure remains fail-closed and continues to use the configured
  lifecycle guard debounce for state transitions.
- The new counter is diagnostic evidence only; it must never grant motion authority,
  mask an invalid current cycle, or introduce an independent permissive threshold.

### R6 - Platform and compatibility constraints

- Core logic remains `no_std`, bounded, allocation-free, and deterministic.
- Existing single-Domain and multi-Domain receive consumers retain their behavior.
- Public documentation and the capability manifest must distinguish completed software
  qualification from open hardware/HIL qualification.

## Acceptance Criteria

- [x] Engine tests prove the observer fires only for a pure WKC mismatch and reports
      exact slot, generation, expected WKC, actual WKC, and header evidence.
- [x] Domain tests prove one increment per due cycle, saturation, actual-WKC retention,
      no process-data commit, good-cycle reset, other-failure reset, and non-due
      preservation.
- [x] Scheduled multi-Domain tests prove only the owning due Domain is updated and
      foreign/control indices do not mutate Domain counters.
- [x] ProcBuf tests prove the counter is projected, the record size/offsets remain
      stable, ABI version 7 has a new layout hash, and version 6 is rejected.
- [x] Lifecycle tests prove current-cycle WKC remains fail-closed and existing guard
      debounce controls the state transition independently of the diagnostic counter.
- [x] Software PRD, master requirements, capability manifest, and Trellis specifications
      describe the implemented boundary and remaining physical qualification gap.
- [x] Formatting, focused tests, `no_std` checks, repository validation, and `make ci`
      pass.
- [ ] The completed task is committed, archived, pushed to `ra1mj/ESOP`, and the exact
      final commit passes GitHub Actions.

## Notes

- Consecutive means consecutive *due Domain cycles with an explicit, fully attributed
  WKC mismatch*. A different receive failure breaks that diagnostic streak.
- The aggregate `CycleReport.wkc_mismatches` remains the current-cycle engine summary;
  the new Domain counter adds ownership and episode history rather than replacing it.
- Verification passed on 2026-09-27 with `cargo test -p esop-ethercat-core
  --all-features`, the focused lifecycle/ProcBuf/Linux quality tests,
  `make test-hil`, and full `make ci`.
