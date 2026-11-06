# EtherCAT Runtime State Requests

## 1. Scope / Trigger

Use this contract when control-plane code explicitly requests one verified
EtherCAT slave to move between INIT, PREOP, SAFEOP, or OP while cyclic Domain
and DC work continues. It covers `request_state` only. Rescan, reconfiguration,
automatic retry, automatic OP return, and application recovery policy remain
separate operations.

## 2. Signatures

```rust
StateRequestController::new()
controller.start(StateRequestConfig { .. })
controller.start_with_op_only(StateRequestConfig { .. }, profile)
controller.next_control_action(now_ns) // StateRequestAction::{Al, OpOnly}
controller.accept_control(action, generation, payload, wkc, now_ns)
controller.start_direct_preop(StateRequestConfig { requested_state: EthercatState::PreOp, .. })
controller.status(handle)
controller.result(handle)
startup.start_state_request(&mut controller, position, target, generation, now_ns, deadline_ns)
services.with_state_request(&mut controller)
```

`StateRequestConfig` freezes position, station address, observed AL status,
target state, generation, absolute deadline, per-request timeout, the complete
transition-timeout profile, and the Device Emulation acknowledgement policy.

## 3. Contracts

- The controller owns one operation, one stable nonzero handle, one inner
  `AlTransitionController`, one existing `OpOnlySyncManagerController`, and
  fixed-size progress/result/fault evidence.
- Multi-step requests start exactly one legal `next_state` step at a time and
  publish the next step only after an exact verified AL status response.
- `start_direct_preop` is reserved for the verified single-slave
  reconfiguration coordinator. It may request PREOP directly from SAFEOP/OP;
  ordinary runtime requests still use legal stepwise traversal and callers
  must not use the direct path as a general ESM shortcut.
- Each step uses the lesser of its ESI/ETG timeout and the remaining overall
  deadline. Each wire request uses the lesser of request timeout and step time.
- `StartupController` is the verified context authority. It admits only Ready,
  online, configured retained records and supplies station/status/profile data.
- Startup binds its retained OpOnly profile to `start_with_op_only`. Disable and
  exactly read back every OpOnly output before AL traversal; after verified OP,
  enable and exactly read back every output before Complete. Same-state requests
  still verify the appropriate activation. Both controllers share the original
  step deadline and overall absolute deadline; preparing AL before SM work does
  not grant a new budget after the SM response.
- Keep ordinary `start`, `next_action`, `accept`, and `timeout` source-compatible
  for AL-only callers. Composite work uses `next_control_action`, `control_pending`,
  `accept_control`, and `timeout_control`; the old AL-only next_action adapter
  returns OpOnlyUnsupported without mutation while SM work is required.
  enqueue_pending/accept_completed and the production scheduler handle both.
- AL observations retain the time of the actual AL read. A later SM readback
  cannot refresh AL age; the result carries its separate completion time.
- Production priority is Startup, PDO/Watchdog/Mapping/DC configuration,
  State Request, Mailbox, then diagnostic Register Request. Domain/DC transport
  runs before the selected service slot.
- One `InFlight` control request is retained across cycles and never resent.
  Completion must match token, index, generation, address, operation, payload,
  response length, deadline, and exact WKC.
- A successful observation is reconciled into Startup retained state using the
  actual master cycle. Active or faulted requests clear the topology lifecycle
  gate; completion does not restore independent drive/DC/command/permit gates.
- Faults latch because an AL write may have reached hardware. Starting another
  request from `Faulted` is forbidden until a higher-level recovery path
  replaces the uncertain evidence.

## 4. Validation & Error Matrix

| Condition | Result |
| --- | --- |
| Controller already transitioning | `Busy` |
| Controller fault is latched | `FaultLatched` |
| Stale or unknown handle | `InvalidHandle` |
| Zero station, elapsed deadline, zero request timeout | typed invalid-input error |
| Unknown or illegal ESM path | `InvalidTransition` |
| Invalid timeout profile | `InvalidTransitionTimeouts` |
| Startup not Ready / record absent / offline / unconfigured | typed retained-context error |
| AL-only action adapter while OpOnly work is required | `OpOnlyUnsupported(position)`; use composite API |
| OpOnly generation/WKC/length/readback/deadline failure | latched `OpOnly(...)`; no Complete |
| Frozen request differs from AL action or pool request | `ActionMismatch` |
| Generation, WKC, payload, deadline, timeout, or AL status failure | latched `Al(...)` or `Control(...)` |
| Retained station/status changes before the next action | latched retained-state mismatch |
| Successful observation cannot be reconciled | latched `RetainedResultMismatch` |

## 5. Good / Base / Bad Cases

- Good: INIT to OP verifies PREOP, SAFEOP, and OP as separate AL write/read
  steps and reconciles each successful observation before the next step.
- Base: an FPWR response arrives one production cycle late; cyclic LRW and DC
  FRMW still run, the request stays `InFlight`, and no duplicate FPWR is sent.
- Bad: submit a second request after an uncertain timeout, duplicate Startup's
  OpOnly sequence, or treat state-request completion as permission for motion.

## 6. Tests Required

- Controller unit tests: immediate completion, multi-step legal traversal,
  transactional invalid submission, busy/faulted behavior, step and overall
  deadlines, action/generation/WKC mismatch, AL error acknowledgement, stale
  handles, result reuse, and latched first faults.
- Startup tests: Ready/online/configured context, unknown position, profile
  timeout, Device Emulation policy, retained OpOnly binding, station/status,
  and successful cycle-bound reconciliation.
- Composite tests: INIT→OP and OP→SAFEOP ordering, same-state verification,
  immutable AL observation age, shared absolute deadline, exact reserved-bit
  readback, first fault and stale handles.
- Scheduler tests: one-slot AL/OpOnly sequence, readback failure, fixed priority,
  no in-flight preemption, exact frozen-action
  matching, terminal consumption, and persistent fault selection.
- Linux integration: assert LRW/FRMW precede FPWR/FPRD, delay one response over
  a cycle boundary, assert no retransmission and one control slot, then verify
  exact result plus fail-closed topology gating.
- Repository: focused tests, `make test-hil`, no_std check, `make ci`, and exact
  GitHub Actions for the pushed commit.

## 7. Wrong vs Correct

Wrong: drive the AL controller directly from an application loop and infer the
station, timeout, or lifecycle result locally.

```rust
al.start(request)?;
port.tx_submit(&frame)?;
topology_valid = true;
```

Correct: obtain verified context from Startup and route the operation through
the production scheduler.

```rust
let handle = startup.start_state_request(
    &mut state_request,
    position,
    target,
    generation,
    now_ns,
    deadline_ns,
)?;
let services = ScheduledProductionServices::new(None, None, None, None)
    .with_state_request(&mut state_request);
```
