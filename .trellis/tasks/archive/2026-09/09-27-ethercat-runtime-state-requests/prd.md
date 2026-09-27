# EtherCAT Runtime State Requests

## Goal

Deliver the first independently verifiable slice of `REC-001`: an explicit,
bounded asynchronous `request_state` operation for one verified EtherCAT slave.
The operation must reuse the existing AL transition protocol authority, run in
the production control-service budget after cyclic Domain/DC work, and expose
typed progress and terminal evidence without automatic retry or OP return.

## Functional Requirements

1. Add an allocation-free single-slot state-request controller with stable
   handles, explicit phase, immutable status, typed progress, result, fault,
   position, station address, requested state, observed status, generation,
   and absolute deadline evidence.
2. Accept a request only when the current and target ESM states are known, the
   transition is legal, transition timeout data is valid, request timeout is
   non-zero, and the absolute deadline is in the future.
3. Advance multi-step state requests through the existing
   `AlTransitionController`; for example INIT to OP must perform PREOP, SAFEOP,
   and OP as separate verified steps.
4. Bound every AL step by both the applicable ESI/ETG timeout class and the
   caller's overall absolute deadline. Bound each wire request by the configured
   request timeout and the remaining step budget.
5. Preserve exact token, datagram index, generation, address, operation,
   payload, response length, deadline, and WKC validation when consuming the
   shared control request.
6. Add a `StartupController` adapter that resolves a verified online/configured
   record, transition timeout profile, request timeout, and Device Emulation
   AL-error acknowledgement policy without rescanning or rebuilding topology.
7. Reconcile each successful final observation into the retained slave table.
   A controller fault remains latched and selected by the scheduler until an
   explicit higher-level recovery path replaces the uncertain evidence.
8. Reject runtime state changes for profiles with non-empty OpOnly output
   SyncManager rules in this child task. Existing startup sequencing remains
   the only authority for those devices until a later recovery integration
   task extracts a reusable OpOnly transition sequence.
9. Add the controller as a production service after startup/configuration/DC
   services and before ordinary mailbox and asynchronous diagnostic register
   requests. It may not preempt an already in-flight service request.
10. Project an active or faulted state request into the topology lifecycle gate
    so motion readiness is revoked before its first wire action. Terminal
    success alone does not bypass drive, health, DC, command-age, or CiA 402
    gates.

## Determinism and Safety Requirements

- No heap allocation, blocking, sleeping, recursion, unbounded polling, or
  automatic retry.
- At most one state-request control datagram is admitted per production cycle;
  accepted in-flight requests are retained across cycles and never duplicated.
- Submission validation must finish before controller mutation where possible;
  invalid requests cannot erase the prior completed result.
- Timeout, WKC mismatch, malformed response, stale generation, request/action
  mismatch, transport failure, retained-state mismatch, or AL error fails
  closed with a typed latched fault.
- The feature is communication/control evidence only and makes no HIL, timing,
  safe-output, STO, ETG conformance, or functional-safety claim.

## Acceptance Criteria

- [x] Public `StateRequestController` APIs and types are `no_std`, fixed-size,
      deterministic, and re-exported from `esop-ethercat-core`.
- [x] Unit tests cover immediate completion, legal multi-step transitions,
      invalid/busy requests, overall and step deadlines, WKC/generation/action
      mismatches, AL errors, latched faults, stale handles, and result reuse.
- [x] Startup adapter tests cover unknown/offline/unconfigured positions,
      profile timeout selection, Device Emulation acknowledgement policy,
      OpOnly rejection, retained station validation, and successful table
      reconciliation.
- [x] Production scheduler tests prove fixed priority, no preemption of an
      in-flight higher-priority request, exact request matching, terminal
      consumption, persistent fault selection, and lifecycle gating.
- [x] Linux simulation proves process `LRW` and DC `FRMW` are transmitted before
      state-request `FPWR/FPRD`, a delayed response spans cycles without
      retransmission, and the shared control pool remains bounded.
- [x] Existing startup, PDO, mailbox, register-request, lifecycle, no_std, and
      generated-product behavior remains compatible.
- [ ] `make test-hil`, `make ci`, focused tests, `git diff --check`, and exact
      GitHub Actions for the pushed commit succeed.

## Out of Scope

- `rescan`, single-slave reconfiguration, automatic retry, or automatic OP
  return; those remain sibling child tasks under the recovery parent.
- Runtime OpOnly SyncManager disable/enable sequencing.
- Changing process-image layout, PDO configuration, station assignment, SII
  evidence, or product identity.
- Real-slave interoperability, target timing qualification, HIL, ETG
  conformance, or functional-safety certification.
