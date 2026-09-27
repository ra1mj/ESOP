# EtherCAT Recovery Integration and Qualification

## Goal

Complete the software-deliverable portion of `REC-001` by giving explicit
`request_state`, `rescan`, and `reconfigure_slave` operations one immutable
public observation surface, fixed-capacity diagnostic events, and cross-operation
cyclic-load evidence. The integration must preserve each existing protocol
controller as the authority and must not introduce automatic recovery or an
implicit return to OP.

## Background

- `StateRequestController`, `StartupController` rescan state, and
  `ReconfigureSlaveController` already expose stable handles plus operation-specific
  status, result, progress, and typed errors.
- `ScheduledProductionServiceScheduler` already runs Domain/DC work first,
  retains one accepted control request across cycles, and selects recovery in
  fixed priority order: Rescan, Reconfigure Slave, then State Request.
- Existing Linux simulations prove cyclic coexistence for each operation and
  already exercise Reconfigure Slave competing with a lower-priority State
  Request. They do not yet expose one common status/result type or common
  recovery event stream.
- `REC-001` must remain a partial product capability after this task because
  target WCET/jitter, physical response provenance, real-slave interoperability,
  long-duration execution, HIL, and ETG conformance remain outside software
  simulation evidence.

## Requirements

1. Add allocation-free, `no_std`-compatible public snapshot enums covering the
   status and result of all three explicit recovery operations without copying
   or replacing their protocol state machines.
2. Provide common snapshot accessors for operation kind, stable sequence,
   coarse phase, generation, absolute deadline, optional target position and
   station address, and exact typed fault evidence where available.
3. Preserve operation-specific status/result values inside the unified types so
   callers do not lose requested ESM state, Startup progress, retained identity,
   AL status, or exact child error details.
4. Add a fixed-capacity, non-blocking diagnostic queue that observes immutable
   snapshots and emits at most one event for each submitted operation and each
   distinct progress, completion, or fault transition. Re-observing an
   unchanged snapshot must not emit a duplicate event.
5. Diagnostic overflow must increment an atomic lost-event counter and must
   never block, allocate, retry a protocol request, or modify recovery
   controller state.
6. Keep cyclic Domain/DC transport and production-service ownership unchanged.
   The integration layer is observational and cannot submit wire work, restore
   lifecycle gates, retry an operation, or return a slave to OP.
7. Extend deterministic tests to cover all unified conversions, diagnostic
   deduplication, fault preservation, ring overflow, mixed recovery priority,
   one-slot control-pool ownership, LRW/FRMW-before-service ordering, delayed
   response non-retransmission, and software deadline/budget reporting.
8. Update requirement, software PRD/release boundary, capability manifest, and
   Trellis specifications consistently. Claims must distinguish completed
   deterministic software behavior from remaining physical qualification.

## Acceptance Criteria

- [x] Public unified status/result/fault types cover State Request, Rescan, and
      Reconfigure Slave and expose common immutable metadata.
- [x] A fixed-capacity recovery diagnostic queue emits submitted, progress,
      completed, and faulted events without duplicate unchanged observations;
      overflow is observable and non-blocking.
- [x] Existing operation-specific handles, statuses, results, errors, and
      scheduling semantics remain source-compatible and authoritative.
- [x] Tests prove Rescan outranks Reconfigure Slave, which outranks State
      Request, while the shared cyclic LRW/DC work still executes first and the
      shared control pool never owns more than one recovery request.
- [x] Tests prove an in-flight recovery request is retained without
      retransmission and that each simulated cycle reports bounded deadline
      evidence without replacing PDO processing.
- [x] Lifecycle behavior remains fail-closed: observation and diagnostics do
      not restore topology, drive, DC, command-age, CiA 402, safety, or motion
      permit gates.
- [x] `REC-001`, the capability manifest, software PRD, release boundary, and
      Trellis specs describe the completed software layer and the remaining
      WCET/HIL/conformance gaps consistently.
- [ ] Focused tests, core `no_std` checks, `make test-hil`, `make ci`, and exact
      GitHub Actions for the pushed commit pass.

## Out of Scope

- A new recovery orchestrator, queue of pending recovery commands, cancellation,
  automatic retry, automatic rescan, automatic reconfiguration, or automatic
  SAFEOP/OP return.
- Changing the fixed production-service priority or allowing parallel recovery
  datagrams.
- Process-image resizing, hot-plug product mutation, multi-slave repair, FSoE,
  STO control, or functional-safety claims.
- Target-hardware WCET/jitter qualification, real-slave/HIL interoperability,
  long-duration qualification, or ETG certification/conformance evidence.
