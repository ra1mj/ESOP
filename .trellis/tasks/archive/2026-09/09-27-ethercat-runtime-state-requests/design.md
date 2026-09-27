# Design: EtherCAT Runtime State Requests

## Components

Add `crates/esop-ethercat-core/src/state_request.rs` with a standalone
`StateRequestController`. The controller owns one `AlTransitionController`, one
accepted operation, one stable handle, the latest verified AL observation, and
one terminal result/fault. It does not own topology, process data, or scheduler
state.

`StartupController::start_state_request` acts as the verified-context adapter.
It resolves the retained `SlaveRecord` and `StartupSlaveProfile`, rejects
non-ready/offline/unconfigured/OpOnly cases, and starts the standalone
controller with copied fixed-size context. `reconcile_state_request` validates
position and station address before updating the retained table after final
success.

## Public Operation Model

```text
Idle / Complete
      |
      | start(config) -> StateRequestHandle
      v
 Transitioning -- next_action/enqueue/accept --> Transitioning
      |                                          (next ESM step)
      +---------------------------------------> Complete(result)
      |
      +-- any uncertainty/error -------------> Faulted(error)
```

`Faulted` is intentionally latched and remains scheduler-active. Because an AL
write may have reached the slave before a timeout or malformed response, a new
request cannot safely assume the retained starting state. The later explicit
rescan/reconfiguration paths provide the evidence needed to recover.

## Stable Handle and Snapshot

Each accepted operation receives a non-zero wrapping sequence handle. Status
and result lookup require the exact handle, preventing callers from consuming a
new operation as if it were an old one. Snapshots are copied value types and
contain no references or formatted strings.

The controller accepts a new operation from `Idle` or `Complete`; starting a
new one invalidates only the prior handle after all new input validation has
succeeded. `Faulted` rejects new operations.

## Multi-Step AL Sequencing

For each step, `next_state(current, target)` selects the required ESM state.
The controller chooses the matching `AlTransitionTimeouts` class and caps it at
the operation's remaining absolute deadline. It starts the existing AL FSM with
the exact observed `AlStatus` and Device Emulation acknowledgement policy.

When AL reports `Reached`, the observation becomes authoritative inside the
controller. If it equals the final target, the controller publishes a result;
otherwise it resets only its inner AL FSM and begins the next legal step. Wire
actions and response validation remain implemented by the existing AL and
control-pool APIs.

## Production Scheduler Integration

Add `StateRequest` to the production service kind/progress/fault model and a
builder on `ScheduledProductionServices`. Priority is:

```text
Startup -> configuration controllers -> StateRequest -> Mailbox -> RegisterRequest
```

The scheduler already runs process Domain and DC work before its service slot.
The new branch follows the same pending-action, request matching, expiry,
terminal consumption, and prepared-request release contract as other control
services. When a Startup controller is supplied, final successful progress is
reconciled before the report is returned.

`StateRequestPhase::Faulted` counts as active so the scheduler keeps reporting
the fault. `Complete` is ready and no longer active after the terminal cycle.

## Lifecycle Projection

`other_cycle_facts_from_production_service_cycle` maps `StateRequest` to the
topology gate. Transitioning or faulted reports clear `topology_valid`; only a
verified terminal success can report the service ready. Independent lifecycle
gates remain mandatory and are not restored by this service.

## OpOnly Boundary

Startup currently composes AL transitions with OpOnly SyncManager disable and
enable operations. Duplicating that private sequencing in this child would
create two protocol authorities. The adapter therefore rejects a real state
change when the selected profile has any OpOnly outputs. A later recovery child
may extract the startup sequence into a shared bounded component before lifting
this restriction.

## Verification

- Module unit tests drive actions directly to cover state and error matrices.
- Startup unit tests use retained profile/table fixtures.
- Production scheduler unit tests cover selection and latched faults.
- Linux scheduled-domain simulation proves `LRW -> FRMW -> FPWR/FPRD` ordering,
  delayed RX retention, bounded control-pool ownership, and lifecycle gating.

## Rollback

The module, service enum variants, optional service binding, Startup adapter,
and lifecycle match arm are additive. Removing them restores the current
runtime without changing existing controller layouts or public behavior.
