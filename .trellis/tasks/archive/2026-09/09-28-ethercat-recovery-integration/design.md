# Design: EtherCAT Recovery Integration and Qualification

## Boundary

The integration is an observational layer over three existing authorities:

```text
StateRequestStatus/Result ----\
RescanStatus/Result -----------+--> ExplicitRecoveryStatus/Result
ReconfigureSlaveStatus/Result-/                 |
                                                  v
                                  ExplicitRecoveryDiagnostics<N>
                                                  |
                                  fixed SPSC diagnostic events
```

No unified controller owns protocol state or submits requests. Applications
continue to start operations through `StartupController` or the operation-specific
controller and continue to schedule them through
`ScheduledProductionServiceScheduler`.

## Unified Snapshots

Add `crates/esop-ethercat-core/src/recovery.rs` with:

- `ExplicitRecoveryKind`: `StateRequest`, `Rescan`, `ReconfigureSlave`.
- `ExplicitRecoveryPhase`: `Idle`, `Active`, `Complete`, `Faulted`.
- `ExplicitRecoveryFault`: a lossless wrapper around the three existing typed
  error enums.
- `ExplicitRecoveryStatus`: variants containing the complete existing status
  values.
- `ExplicitRecoveryResult`: variants containing the complete existing result
  values.

`From` conversions remain additive. Common `const` accessors project kind,
sequence, coarse phase, generation, deadline, optional position/station, and
fault. Operation-specific detail remains accessible through enum matching and
is never reformatted or allocated.

The coarse phase is intentionally observational. Reconfiguration's internal
PDO/watchdog/mapping/DC phases all map to `Active`; the full
`ReconfigureSlavePhase` remains inside its status variant.

## Diagnostic Events

`ExplicitRecoveryDiagnostics<const EVENTS: usize>` owns:

- a fixed SPSC ring of `ExplicitRecoveryDiagnosticEvent`;
- one last-observed status slot per recovery kind;
- an atomic lost-event counter.

`observe(cycle, timestamp_ns, status)` compares the new immutable status with
the last status for that operation kind:

- a new operation sequence in a nonterminal phase emits `Submitted`;
- a changed snapshot for the same active operation emits `Progress`;
- a first observed complete snapshot emits `Completed`;
- a first observed faulted snapshot emits `Faulted`;
- an identical snapshot emits nothing.

The event contains the original unified status, so exact typed faults and
operation-specific detail remain available. Queue overflow records one loss
and advances the remembered observation so an unchanged status cannot amplify
overflow on every cycle; it never alters a protocol controller.

## Scheduling and Budget Evidence

The production scheduler remains unchanged. Qualification tests observe its
existing cycle report and assert:

1. cyclic LRW and DC FRMW precede the selected recovery datagram;
2. priority is Rescan, Reconfigure Slave, then State Request;
3. the fixed control pool has at most one owned request;
4. an `InFlight` request survives a cycle boundary without retransmission;
5. `post_receive_deadline_met` remains true for qualified simulated cycles;
6. recovery completion does not synthesize independent lifecycle readiness.

Existing per-operation Linux simulations are extended instead of duplicating
their large Startup and virtual-port fixtures. The mixed reconfiguration/state
request simulation remains the cross-operation transport proof; scheduler
tests cover the Rescan priority edge, and unified diagnostic tests cover all
three snapshot variants.

## Compatibility

- All new APIs are additive exports from `esop-ethercat-core`.
- Existing operation-specific APIs remain the source of truth.
- No heap allocation, formatting, trait objects, locks, sleeps, or unbounded
  loops enter the cyclic path.
- The module remains available in `no_std` builds.

## Documentation and Claim Boundary

Update `REC-001`, the ETG/CiA 402 requirements, the software PRD R2 section,
`capability_manifest.json`, and a new Trellis recovery-integration spec.
Software capability stays `partial`: this task closes the unified API and
deterministic mixed-load evidence but not physical timing, HIL, interoperability,
ETG conformance, safe outputs, STO, or functional-safety qualification.

## Rollback

The new module and its tests can be reverted without changing the three
controllers or the scheduler. Documentation reverts with the module. No data
migration or generated ABI update is required.
