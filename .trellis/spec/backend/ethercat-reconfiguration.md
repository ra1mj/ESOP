# EtherCAT Single-Slave Reconfiguration

## 1. Scope / Trigger

Use this contract when a control-plane caller explicitly reconfigures exactly
one retained EtherCAT slave while cyclic Domain and DC traffic continues. The
operation reuses verified Startup/product evidence, stops in PREOP, and never
restores motion permission. Rescan, process-image resizing, multi-slave repair,
automatic retry, and automatic SAFEOP/OP return are separate operations.

## 2. Signatures

```rust
StaticProductConfig::build_reconfigure_slave_plan::<SMS, FMMUS, OPS>(
    position,
    retained_identity,
    frozen_mapping,
)
startup.start_reconfigure_slave(
    &mut controller,
    plan,
    generation,
    now_ns,
    deadline_ns,
    application_time_ns,
)
controller.status(handle)
controller.result(handle)
services.with_reconfigure_slave(&mut controller)
```

The fixed-capacity public contract is `ReconfigureSlavePlan`,
`ReconfigureSlaveContext`, `ReconfigureSlaveHandle`, `ReconfigureSlavePhase`,
`ReconfigureSlaveStatus`, `ReconfigureSlaveProgress`,
`ReconfigureSlaveResult`, and `ReconfigureSlaveError`.

## 3. Contracts

- Product code builds one bounded plan from the generated position/station,
  retained identity, generated mailbox/PDO/watchdog/DC policy, and the caller's
  frozen mapping. Plan construction performs no runtime heap allocation.
- Startup is the admission authority. It validates Ready state, online and
  configured target status, exact position/station/identity/mailbox evidence,
  matching verified SyncManager/FMMU banks, profile ownership, future absolute
  deadline, and controller availability before mutating retained state.
- Successful admission clears only the target's `configured` flag before the
  first action is exposed. Preserve every unrelated record and all retained
  topology, product, mailbox, register, identity, and DC evidence.
- Disable and exactly read back every target OpOnly output SyncManager before
  lowering AL state. Reconfiguration uses the dedicated direct-PREOP path: it
  may request PREOP from SAFEOP or OP, but it must never request SAFEOP or OP
  and must not re-enable OpOnly outputs.
- Configuration order is fixed: PDO mailbox download/upload verification,
  optional watchdog write/readback, SyncManager/FMMU clear/write/readback,
  target DC clock, then target DC Sync. Empty phases advance without inventing
  evidence.
- Every configuration write addresses the target station. Targeted DC Sync may
  read the retained reference-clock station when it differs from the target;
  no write may address the reference or another unrelated station.
- One absolute deadline caps every child timeout, wire request, and mailbox
  transaction. The first fault is retained, the target remains unconfigured,
  no retry occurs, and only a new explicit operation can replace the handle.
- Production priority is Startup, Rescan, Reconfigure Slave, ordinary
  configuration/DC services, State Request, Mailbox, then Register Request.
  Domain/DC cyclic transport still runs first. One accepted request remains
  owned across cycles and is never retransmitted because its response is late.
- Success requires all applicable exact readbacks plus an observed PREOP status
  without AL error. Startup commits only the target back to configured PREOP.
  Completion does not restore OP or bypass drive, DC, command-age, supervisor,
  CiA 402, safety, or permit gates.
- Active or faulted reconfiguration closes lifecycle topology readiness. A
  completed operation contributes only the target's communication/configuration
  evidence and does not synthesize unrelated readiness.

## 4. Validation & Error Matrix

| Condition | Result |
| --- | --- |
| Startup not Ready, target absent/offline/unconfigured | `StartupNotReady`, `UnknownPosition`, `SlaveOffline`, or `SlaveUnconfigured` |
| Controller already active | `Busy`, with no record mutation |
| Stale handle or nonterminal result request | `InvalidHandle` or `ResultNotReady` |
| Zero station, elapsed deadline, zero request timeout | typed invalid-input error |
| Retained station, identity, mailbox, register bank, profile, or state mismatch | typed mismatch error before target invalidation |
| PDO without mailbox, invalid mailbox/watchdog, or missing required DC topology | typed plan/controller error before action publication |
| OpOnly write/readback or direct PREOP transition failure | first wrapped OpOnly/StateRequest fault |
| PDO/watchdog/mapping/DC action, generation, WKC, payload, readback, or timeout failure | first wrapped child fault; target stays unconfigured |
| Whole-operation deadline reached | `OperationDeadlineExceeded`; no retry |
| Completion status is not exact PREOP or retained target drifted | `RetainedStateMismatch`; do not commit configured |

## 5. Good / Base / Bad Cases

- Good: slave 0 is admitted from OP, its configured flag clears, OpOnly outputs
  disable, direct PREOP is verified, all target configuration readbacks pass,
  and only slave 0 is committed configured in PREOP.
- Base: one target FPWR response arrives in the next production cycle. LRW and
  FRMW run in both cycles, the one control slot stays owned, and no duplicate
  FPWR is sent.
- Bad: construct a fresh plan from guessed live values, invalidate all slaves,
  step OP through SAFEOP, write the non-target reference clock, retry a timeout
  automatically, or interpret completion as motion permission.

## 6. Tests Required

- Core coordinator: transactional start, stable/current/stale handles, phase
  snapshots, direct PREOP payload, first-fault retention, whole-operation
  deadline, PDO/watchdog/mapping order, and exact target station writes.
- Startup/product: unknown position, inconsistent evidence, expired deadline,
  busy controller, insufficient plan capacity, target-only invalidation, and
  target-only PREOP commit.
- DC: target clock writes only the selected slave; target Sync reads a
  non-target reference and writes only the selected slave.
- Scheduler/Linux: LRW/FRMW first, reconfiguration priority over state request,
  one bounded slot, delayed response without retransmission, unchanged
  unrelated record, target configured PREOP completion, and no gate bypass.
- Lifecycle: active/faulted closes topology; successful completion does not
  restore OP or independent drive/DC/command/CiA 402 readiness.
- Repository: focused tests, core `no_std` check, `make test-hil`, `make ci`,
  and exact GitHub Actions success for the pushed commit.

## 7. Wrong vs Correct

Wrong: invalidate topology globally and call ordinary stepwise state requests
plus configuration controllers from an application loop.

```rust
startup.clear_all_readiness();
state_request.start(op_to_safeop)?;
state_request.start(safeop_to_preop)?;
mapping.start(unverified_bank, guessed_mapping)?;
```

Correct: build from frozen product/Startup evidence and route the one operation
through the named production service.

```rust
let plan = product.build_reconfigure_slave_plan(
    position,
    retained_identity,
    frozen_mapping,
)?;
let handle = startup.start_reconfigure_slave(
    &mut reconfigure,
    plan,
    generation,
    now_ns,
    deadline_ns,
    application_time_ns,
)?;
let services = ScheduledProductionServices::new(Some(&mut startup), None, None, None)
    .with_reconfigure_slave(&mut reconfigure);
let status = reconfigure.status(handle)?;
```
