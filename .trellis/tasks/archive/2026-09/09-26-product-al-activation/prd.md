# Orchestrate Product EtherCAT AL Activation

## Goal

Add a bounded, fail-closed PREOP configuration barrier to the existing startup
and production-service path so a product can enter PREOP, finish its required
PDO/SM-FMMU/DC services, and only then continue through legal SAFEOP and OP
transitions without rescanning or accepting caller-supplied readiness booleans.

## Background

- `StartupController` already scans, assigns fixed addresses, verifies exact
  identities, and performs one legal AL step at a time until a requested state.
- It currently transitions straight through PREOP, SAFEOP, and OP, so scheduled
  PDO/mapping/DC configuration has no explicit window inside that startup run.
- `ScheduledProductionServiceScheduler` already owns fixed-priority service
  selection, request identity, cross-cycle request ownership, and lifecycle
  readiness projection for Startup, PDO Configuration, Mapping, DC, and Mailbox.
- The scheduler can inspect controller phases directly; no new untyped
  readiness input is needed.

## Requirements

### R1. Optional PREOP configuration barrier

1. Extend `StartupConfig` with an opt-in fixed set of required configuration
   services: PDO Configuration, Mapping, and/or DC Configuration.
2. Existing `StartupConfig::new(target_state)` callers shall retain the current
   uninterrupted behavior when no service is required.
3. A barrier is valid only when the final target is SAFEOP or OP; INIT, PREOP,
   or Unknown targets with required configuration shall fail before startup
   state mutates.

### R2. All-slave PREOP entry

1. With a barrier enabled, scan and identity verification remain unchanged.
2. Every expected slave shall reach and confirm PREOP before the startup FSM
   exposes the configuration window.
3. The startup table and verified identities shall remain intact; activation
   must not rescan or reread SII after configuration completes.
4. Startup shall expose an explicit `AwaitingConfiguration` phase and progress
   event while returning no AL action.

### R3. Scheduler-owned release

1. While Startup awaits configuration, it shall temporarily yield priority so
   the existing order PDO Configuration -> Mapping -> DC Configuration can run.
2. The scheduler shall release the barrier only when every service required by
   the frozen Startup configuration is present and its real controller phase is
   Complete.
3. A missing required binding shall return a typed scheduler error. Idle,
   active, waiting, retrying, or faulted required services shall not release the
   barrier.
4. A faulted required service shall retain its current fail-closed priority and
   require explicit controller restart as today.

### R4. Post-configuration AL progression

1. After scheduler release, Startup shall reuse its verified slave table and
   transition each slave from PREOP through SAFEOP to the frozen final target.
2. Each write and status poll shall continue through the existing control pool,
   shared Domain/DC RX, generation checks, deadlines, and exact AL status-code
   handling.
3. Startup shall become Ready only after every slave reports the final target.
4. For an OP target, an observed SAFEOP step is mandatory before OP; no state
   skip or synthetic completion is allowed.

### R5. Reporting and lifecycle behavior

1. Production reports shall distinguish waiting at the configuration barrier
   from transport waiting or request rebuild.
2. Topology/startup readiness remains false while at the barrier and becomes
   true only after final SAFEOP/OP completion.
3. PDO, Mapping, or DC reports continue to drive the Configuration gate from
   their actual controller state; caller readiness overrides remain forbidden.
4. AL error code, timeout, request mismatch, missing service, and configuration
   service faults shall remain observable as typed failures.

### R6. Compatibility and bounded execution

1. Preserve `no_std`, allocation-free, fixed-capacity, non-blocking execution.
2. Do not add threads, sleep, dynamic dispatch, heap storage, or duplicate AL,
   mailbox, mapping, or DC protocol code.
3. Existing startup and scheduler call sites without a barrier shall compile
   and retain behavior.
4. Explicit restart shall clear a prior startup fault/barrier and use only the
   newly supplied configuration.

### R7. Qualification boundary

1. Software simulation proves ordering, ownership, AL-state polling, and
   fail-closed gating only.
2. This task does not generate multi-slave configuration batches, discover
   mailbox/SM/FMMU/DC descriptors, authenticate physical readback, or prove
   process-data WKC across a full hyperperiod.
3. Physical slave interoperability, timing/WCET, target resources, HIL, and
   functional-safety qualification remain open.

## Acceptance Criteria

- [x] A barrier-enabled one-slave flow reaches PREOP, yields to required PDO,
      Mapping, and DC services, then confirms SAFEOP and OP without rescanning.
- [x] Multiple expected slaves all confirm PREOP before any configuration
      service is allowed to release startup, and all confirm the final target.
- [x] Startup cannot resume while any required service is missing, Idle,
      active, retrying, or faulted; only all-Complete controller state releases.
- [x] Fixed priority remains Startup -> PDO Configuration -> Mapping -> DC
      Configuration -> Mailbox outside the deliberate barrier yield.
- [x] Existing no-barrier startup tests and production-service callers remain
      source-compatible and behaviorally unchanged.
- [x] Startup timeout, AL error code, missing required service, configuration
      fault, and explicit restart are covered by fail-closed tests.
- [x] Lifecycle projection keeps startup/topology unready at PREOP barrier and
      marks it ready only after the final requested AL state.
- [x] Core, Linux simulated-port, lifecycle, `no_std`, Clippy, full CI, BPF,
      capability-manifest, and Zenoh gates pass.
- [x] README, PRD, EtherCAT requirements, product configuration, capability
      claims, and Trellis specs describe the delivered software boundary and
      retain multi-slave batch/HIL/WCET limitations.

## Out of Scope

- Generated arrays of per-slave mailbox, mapping, and DC activation jobs.
- Automatic restart of one service controller for the next slave/config batch.
- Full-period process-data WKC qualification before OP.
- AL error acknowledgement/device-emulation special handling.
- Physical hardware, production timing, certification, or functional safety.
