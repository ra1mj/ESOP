# Explicit CiA 402 Maintenance Fault Reset

## 1. Scope / Trigger

Use this contract to clear a drive's PDS Fault through the existing production
PDO owner while MLG remains inhibited. Clearing an injected fault source is not
PDS Fault Reset. Reset authority cannot enable operation, issue a setpoint,
clear an MLG fault, qualify gates, or renew a motion permit.

## 2. Signatures

```rust
use esop_lifecycle_guard::maintenance::{FaultResetController, FaultResetRequest};
let mut reset = FaultResetController::new(boot_id, 3)?;
reset.submit(&guard, FaultResetRequest {
    boot_id, sequence, transition_sequence, axis_mask, deadline_ns,
}, now_ns)?;
// After the real scheduler RX and owner.complete_service_cycle(...):
let outcome = context.run_service_cycle_with_fault_reset_until(
    &owner, &domains, &service, motion_domain_id, &outputs, &mut reset,
    cycle_deadline_ns,
)?;
owner.settle_output(&outputs, &outcome, next_rx_deadline_ns)?;
```

`FaultResetStatus` exposes phase, immutable request, pulse attempt count,
accepted pulse/zero cycles and terminal typed error. `StopCycleOutcome` carries
optional `maintenance_reset` status alongside its normal transport/publication
results. The module requires EtherCAT, CiA 402 and ProcBuf features.

## 3. Contracts

- Submit an explicit nonzero increasing sequence bound to the controller and
  guard boot, current lifecycle transition, nonempty allowed axis mask and
  future absolute deadline. MLG must be Maintenance or FaultLatched, have no
  retained permit, no armed motion axes and no pending maintenance stop.
- The entry requires the actual `ScheduledProductionCycleOwner` in
  OutputPending for this receive cycle/generation, the same frozen schedule and
  motion Domain, and the exact production service report. It reuses the common
  cycle algorithm, safe-frame builder, index ownership and output handoff.
- Validate the entire current completed Domain, required raw RX facts, selected
  PDS Fault states, all writable Controlword/mode fields, complete image bounds,
  and output aliases before any auxiliary or motion TX. Unknown/stale inputs,
  Fault Reaction Active and merely disappeared fault sources cannot authorize
  a reset pulse.
- A private transactional preview permits only Controlword 0x0080 on selected
  Fault axes and Disable 0 on every other axis. No target field is written.
  Full mapping coverage and immutable frame deadlines remain mandatory.
- Only port acceptance commits a pulse. A failed build/TX cannot claim an
  accepted reset or advance the pulse phase. Pulse attempts are bounded by the
  frozen 1..8 limit and absolute deadline, with at most one attempt per receive
  cycle. Exhaustion requires a new explicit sequence. Expired pending work
  cannot permanently block a later explicit request.
- After an accepted pulse, ZeroRequired forbids another reset request. The
  next maintenance entry emits only Disable 0, accepts fresh feedback even if
  PDS Fault has cleared, and completes only after that zero frame is accepted.
  Failed zero submissions retain the obligation; these are ordinary bounded
  per-cycle inhibited outputs, never repeated reset pulses. Pulse expiry does
  not prohibit the required safe zero frame.
- State projects the actual accepted Controlword (0x0080 or 0). Reset is not
  stop issuance: `axis_stops.issued_action` remains zero. Failed TX preserves
  earlier accepted-controlword truth and still publishes normal faulted State.
- Neither accepted pulse nor observed Fault disappearance clears MLG. Fresh
  stable gates, explicit `clear_fault`, and a post-recovery permit are still
  required before motion. An accepted late reset is reported as accepted;
  normal cycle-deadline failure handling remains active.

## 4. Validation & Error Matrix

| Condition | Result before reset TX |
| --- | --- |
| Wrong boot / stale sequence / transition | `BootMismatch` / `SequenceReplayed` / `TransitionMismatch` |
| Empty/outside mask / Active or Stopping | `InvalidAxisMask` / `InvalidLifecycle` |
| Retained permit / armed or pending stop | `PermitPresent` / `StopPending` |
| Missing/expired request / exhausted attempts | `NoRequest` / `Deadline` / `AttemptsExhausted` |
| Same receive cycle attempted again | `CycleReplayed` |
| Stale or incomplete Domain / selected axis not Fault | `UnverifiedInput` / `NotInFault(axis)` |
| Incomplete/overlapping writable mapping | `InvalidMapping(axis)` / `OverlappingOutputs` |
| Wrong production owner/report/generation | `StopCycleError::ServiceCycleMismatch` |
| Build/TX failure | Normal failed transport result; no pulse acceptance |

## 5. Good / Base / Bad Cases

- Good: current Fault PDO -> explicit request -> accepted 0x0080 -> later fresh
  feedback -> accepted 0 -> Complete, with MLG still FaultLatched.
- Base: port rejects the pulse; request remains unissued and bounded retries
  cannot silently become a successful reset.
- Bad: clear the simulator fault input and rearm directly. The actual PDS Fault
  must first receive the pulse, and MLG recovery remains independently explicit.
- Bad: emit reset through a raw PDO write outside the scheduled owner. The only
  public reset TX entry is owner-bound and publishes the existing State evidence.

## 6. Tests Required

- `maintenance` unit tests: request bindings/lifecycle, current Fault evidence,
  invalid mappings, expiry, replay, attempt limits, transactional preview,
  mandatory zero and no implicit guard mutation.
- `tests/maintenance_reset.rs`: actual production scheduler/owner pipeline,
  one 0x0080 pulse, accepted zero, actual ProcBuf Controlword, unissued stop
  evidence, wrong-owner rejection before TX, failed TX truth, stable-window
  rejection followed by explicit lifecycle fault clear.
- Stateful runner `drive_fault`: real virtual ESC/PDS model consumes 0x0080,
  zero completes, and only explicit recovery plus a new permit restores motion.

## 7. Wrong vs Correct

Wrong: teach a simulator to erase PDS Fault when an injection disappears, or
allow `DriveRequest::FaultReset` through the ordinary inhibited motion branch.

Correct: use a separate explicit maintenance request, current PDS Fault proof,
one accepted reset pulse followed by accepted zero, and the unchanged MLG
fault-clear/new-permit sequence.
