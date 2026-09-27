# Implementation Plan: EtherCAT Explicit Rescan

## 1. Add the Named Rescan Contract

- [x] Add fixed-size handle, phase, status, progress, result, and error types.
- [x] Store rescan operation metadata and stable handle generation in
      `StartupController`.
- [x] Add transactional `start_rescan`, status, result, active/fault helpers,
      and public re-exports.

Validation:

```bash
cargo test -p esop-ethercat-core rescan -- --nocapture
cargo check -p esop-ethercat-core --target aarch64-unknown-none
```

## 2. Reuse Startup with Bounded Invalidation

- [x] Reuse the retained expected/profile arrays and existing Startup reset
      path without duplicating scan/SII/topology protocols.
- [x] Override only target state and configuration barrier policy so rescan
      terminates at PREOP.
- [x] Clear all readiness-bearing evidence before the first action.
- [x] Cap every nested timeout and AL/OpOnly step by the absolute rescan
      deadline while leaving ordinary cold startup unchanged.
- [x] Add unit tests for transactional validation, invalidation, deadline,
      retry, stale handle, PREOP, and first-fault behavior.

## 3. Integrate Production Scheduling and Lifecycle

- [x] Add distinct Rescan service/progress/fault projections at Startup
      priority using the same Startup wire actions.
- [x] Preserve exact request matching, in-flight ownership, timeout, terminal
      consumption, and first-fault selection.
- [x] Extend lifecycle topology gating and add scheduler tests for priority,
      no preemption, terminal success/fault, and no automatic trigger.

## 4. Add Linux Cyclic Coexistence Evidence

- [x] Simulate LRW/FRMW plus a rescan APRD probe with a response delayed across
      a cycle boundary.
- [x] Assert ordering, no retransmission, one control slot, named progress,
      PREOP/no-OP behavior, and topology fail-closed/restoration evidence.

## 5. Update Contracts and Deliver

- [x] Add the backend rescan spec and update its index/quality guidance.
- [x] Update requirements, software PRD, and capability boundary without
      claiming all of `REC-001` complete.
- [x] Run focused tests, no_std, HIL simulation target, formatting, Clippy,
      complete CI, capability validation, and diff checks.
- [x] Complete/archive the child, update the parent checklist and journal,
      commit, push, and wait for exact GitHub Actions success.

## Risk and Rollback Points

- The highest-risk edit is Startup timeout propagation. Keep the no-deadline
  cold-start path unchanged and prove it with the existing full suite.
- Scheduler classification must never create a second owner for a Startup
  action. Rescan metadata observes the Startup FSM; it does not enqueue a
  separate request.
- If integration fails, revert the additive rescan metadata/projection while
  retaining the existing Startup reset authority unchanged.
