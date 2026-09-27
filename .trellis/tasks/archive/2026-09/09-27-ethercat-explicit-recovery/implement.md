# Implementation Plan: EtherCAT Explicit Recovery Control Plane

## 1. Runtime State Requests

- [x] Create and complete child task `ethercat-runtime-state-requests`.
- [x] Add a bounded explicit state-request controller over the existing AL
      transition authority.
- [x] Reconcile verified retained slave state and integrate with production
      scheduling/lifecycle gates.
- [x] Prove multi-cycle coexistence with cyclic PDO/DC traffic.

Validation:

```bash
cargo test -p esop-ethercat-core state_request
make test-hil
make ci
```

## 2. Explicit Rescan

- [x] Create and complete child task `ethercat-explicit-rescan`.
- [x] Add named rescan submission/progress/result APIs.
- [x] Invalidate stale readiness before reusing scan/SII/topology startup work.
- [x] Prove cyclic coexistence and no implicit trigger on ordinary faults.

## 3. Single-Slave Reconfiguration

- [x] Create and complete child task `ethercat-single-slave-reconfigure`.
- [x] Build a targeted verified configuration plan.
- [x] Reuse SM/FMMU/PDO/DC controllers without mutating unrelated slaves.
- [x] Stop before automatic OP entry and retain typed evidence.

## 4. Recovery Integration and Qualification

- [x] Create and complete child task `ethercat-recovery-integration`.
- [x] Unify public status/result snapshots and diagnostic events.
- [x] Add mixed cyclic-load ordering and bounded-budget simulations.
- [x] Update requirement, capability, PRD, release-boundary, and Trellis spec
      documentation consistently.
- [x] Re-run complete quality gates and exact GitHub Actions.

## 5. Parent Closeout

- [x] Review all child acceptance evidence against `REC-001`.
- [x] Mark parent acceptance criteria complete without claiming physical timing,
      HIL, ETG conformance, or functional-safety qualification.
- [x] Archive the parent task and record final journal evidence.
