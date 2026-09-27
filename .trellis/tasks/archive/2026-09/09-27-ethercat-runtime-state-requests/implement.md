# Implementation Plan: EtherCAT Runtime State Requests

## 1. Add the Bounded Controller

- [x] Define handle, config, phase, status, observation, result, progress, and
      error value types.
- [x] Implement transactional submission and multi-step AL sequencing.
- [x] Implement exact pending action, control-pool enqueue/consume, timeout,
      fault latching, stable status, and result lookup.
- [x] Re-export the public API from `esop-ethercat-core`.

Validation:

```bash
cargo test -p esop-ethercat-core state_request
cargo check -p esop-ethercat-core --target aarch64-unknown-none
```

## 2. Add Verified Startup Adapters

- [x] Resolve only Ready, online, configured retained records.
- [x] Copy profile timeouts, request timeout, station address, current status,
      and Device Emulation acknowledgement policy.
- [x] Reject non-empty OpOnly profiles for real transitions.
- [x] Reconcile final position/station/target/status into the retained table.
- [x] Add focused adapter tests.

## 3. Integrate Production Scheduling

- [x] Add optional service binding and fixed priority.
- [x] Extend exact request matching, enqueue, terminal consume, fault, and ready
      projections.
- [x] Reconcile final progress when Startup retained state is present.
- [x] Add scheduler unit tests for priority, persistence, and fault behavior.

## 4. Bind Lifecycle and Linux Simulation

- [x] Map active/faulted state requests to topology readiness.
- [x] Add simulation for cyclic PDO/DC ordering and delayed state-request RX.
- [x] Assert no duplicate send, fixed control-pool ownership, exact result, and
      lifecycle fail-closed behavior.

## 5. Update Contracts and Deliver

- [x] Update EtherCAT requirements implementation notes, software PRD,
      capability boundary, and Trellis backend spec without marking all of
      `REC-001` complete.
- [x] Run focused tests, HIL simulation target, formatting, Clippy, no_std,
      complete CI, and diff checks.
- [ ] Complete and archive the child task, update the parent checklist and
      journal, commit, push, and wait for exact GitHub Actions success.
