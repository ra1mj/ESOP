# Design: EtherCAT Single-Slave Reconfiguration

## Architecture

The feature is a fixed-capacity coordinator over existing protocol
authorities. It does not introduce a second implementation of AL transitions,
mailbox/PDO configuration, register mapping, watchdog, or DC setup.

```text
application request
       |
       v
StartupController::start_reconfigure_slave
  validate/freeze plan -> invalidate target configured evidence
       |
       v
ReconfigureSlaveController
  OpOnly disable -> PREOP -> PDO -> watchdog -> mapping -> DC clock -> DC Sync
       |
       v
ScheduledProductionService::ReconfigureSlave
  Domain/DC first -> one retained control/mailbox action -> exact completion
       |
       v
target configured in PREOP | first fault retained and target unconfigured
```

## Public Contracts

### Plan

`ReconfigureSlavePlan` is caller-frozen, fixed-capacity operation input. It
contains:

- target position, verified station address, and retained identity;
- verified mailbox configuration and optional per-slave PDO startup plan;
- the target's frozen `MappingTable`;
- optional single-entry watchdog configuration;
- optional target DC clock/DC Sync entries plus validated retained topology,
  reference station, and common repeat-period evidence;

`ReconfigureSlaveContext` separately freezes generation, observed AL status,
request/transition timeouts, absolute deadline, OpOnly profile, verified
register banks, retained DC topology, and application time.

Product configuration provides a per-position builder that reuses its existing
PDO builder and filters watchdog/DC plans to the selected slave. The caller
continues to provide the mapping table because it is already the runtime's
frozen process-image authority.

### Controller

`ReconfigureSlaveController` owns exactly one operation. Its public surface
provides:

- nonzero monotonically changing operation handle;
- `Idle`, active phase, `Complete`, and `Faulted` status;
- immutable status including position, station, phase, deadline, latest AL
  observation, and first fault;
- one retained terminal result or first typed fault;
- bounded `next_action`/`complete_action` style integration matching the
  existing production service contracts.

Proposed phases are:

```text
Idle
DisablingOpOnly (optional)
MovingToPreOp
ConfiguringPdo (optional)
ConfiguringWatchdog (optional)
ConfiguringMapping
ConfiguringDcClock (optional)
ConfiguringDcSync (optional)
Complete | Faulted
```

The coordinator delegates actions and completions to child controllers. It
does not clone their protocol parsing or validation rules.

## Admission and Evidence Ownership

`StartupController` remains the retained slave/topology authority.
`start_reconfigure_slave` performs a two-stage admission:

1. resolve and validate every retained/product/plan dependency without
   mutation;
2. initialize the coordinator successfully, then synchronously set only the
   target `SlaveRecord.configured` to false before returning success.

Add narrow Startup-owned methods for target configuration invalidation and
successful commit. No caller receives mutable access to `SlaveTable`. On
success Startup commits only the target as configured and records verified
PREOP observation/cycle. On any terminal fault it deliberately leaves the
target unconfigured.

Startup may remain internally `Ready` because unrelated topology evidence is
still valid. Lifecycle adds the named reconfiguration service to topology
readiness, so active/faulted reconfiguration closes the global motion gate.

## Phase Data Flow

### OpOnly and PREOP

When the retained profile contains OpOnly SyncManagers, run
`OpOnlySyncManagerController` to disable and read back those channels. Then
start `StateRequestController` directly with copied verified context and target
PREOP. The Startup convenience state-request gate is not reused because it
correctly rejects nonempty OpOnly profiles before they are disabled.

### PDO and mapping

The PDO phase runs `PdoConfigController` through the existing mailbox
transport. The mapping phase runs `MappingConfigController` with the retained
verified SM/FMMU register banks, clearing and verifying only target descriptors
before writing and reading back the frozen target mapping.

### Watchdog and DC

Watchdog uses a single-entry plan. Add targeted entry points to
`DcClockController` and `DcSyncController`:

- validate against the full retained DC topology/reference authority;
- copy only the target write entry into fixed local state;
- preserve full-plan common repeat-period validation;
- allow DC Sync to read the retained reference station even when it differs
  from the target;
- never generate writes for a non-target station.

These targeted APIs are also useful independently and retain existing full-
startup entry points unchanged.

## Deadline and Error Semantics

The coordinator owns one absolute deadline. Before starting or advancing a
child phase it computes a nested deadline capped by the operation deadline.
An expired operation faults before exposing another wire action.

Errors identify operation phase and preserve the child error category. The
first fault is immutable. There is no cleanup traffic, retry, or compensating
return to OP because retained state may no longer be trustworthy after the
first failed wire action.

## Production Scheduling

Add `ScheduledProductionServiceKind::ReconfigureSlave` with priority:

```text
Startup -> Rescan -> ReconfigureSlave -> configuration services
        -> StateRequest -> Mailbox -> RegisterRequest
```

Domain/DC submission and receive finalization remain ahead of all service
work. The scheduler exposes one named reconfiguration operation while the
coordinator selects the active child transport:

- PDO uses mailbox transport;
- OpOnly, AL, watchdog, mapping, and DC use control transport.

Once a request is accepted, its action identity remains owned until exact RX
completion or timeout. Phase changes occur only after the accepted completion,
so delayed responses cannot cause retransmission.

## Lifecycle and Diagnostics

Lifecycle maps active or faulted reconfiguration to the topology readiness
gate. Terminal success merely removes the reconfiguration-specific block after
the target record is committed; all other lifecycle gates remain authoritative.
Snapshots are allocation-free and suitable for later diagnostics integration.

## Compatibility and Rollback

All public changes are additive. Existing full-startup DC APIs, configuration
controllers, scheduler behavior without an active reconfiguration, and startup
flows remain unchanged. The feature can be reverted by removing the new
coordinator/service and targeted DC entry points without changing the wire
authorities themselves.

## Verification Strategy

- Unit tests drive every coordinator phase and terminal error.
- Targeted DC tests prove reference reads and target-only writes.
- Startup tests compare unrelated retained evidence before and after admission,
  success, and failure.
- Scheduler tests cover service priority, in-flight retention, timeout, and
  one-slot admission.
- Linux simulation combines LRW/FRMW cyclic traffic with delayed multi-cycle
  reconfiguration responses.
- Full quality gates retain `no_std`, release, Clippy, generated-artifact, BPF,
  host integration, and privileged eBPF runtime checks.
