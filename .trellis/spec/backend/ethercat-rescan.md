# EtherCAT Explicit Rescan

## 1. Scope / Trigger

Use this contract only when control-plane code explicitly asks the retained
EtherCAT product plan to verify the network again while cyclic Domain and DC
work continues. Rescan invalidates prior readiness synchronously, reuses the
existing Startup protocol authorities, and stops at verified PREOP. It does
not resize the process image, reconfigure PDO/DC registers, return slaves to
SAFEOP/OP, refresh motion commands, or run automatically after another fault.

## 2. Signatures

```rust
startup.start_rescan(generation, now_ns, deadline_ns)
startup.rescan_status(handle)
startup.rescan_result(handle)
startup.rescan_phase()
startup.active_rescan_handle()
```

`RescanHandle`, `RescanPhase`, `RescanStatus`, `RescanProgress`,
`RescanResult`, and `RescanError` are fixed-size `no_std` values. The retained
Startup expected-slave and profile arrays are the immutable operation plan.

## 3. Contracts

- Submission is allowed only from Ready or Faulted Startup with a future
  absolute deadline and no active rescan. Validate before replacing the prior
  handle/result or clearing retained evidence.
- Successful submission synchronously resets the scan controller, slave table,
  SII/mailbox/register/DC evidence, selected reference, topology, AL/OpOnly
  state, configuration barrier, and prior Startup fault before returning.
- Run the normal Startup pipeline with the retained plan, `target_state =
  PREOP`, and no external configuration services. Do not create a second scan,
  SII, AL, request, or control-pool owner.
- Cap every nested operation timeout, AL/OpOnly step, and wire-request timeout
  by `deadline_ns - now_ns`. Normal cold Startup keeps its configured timeout
  behavior because it has no operation-wide deadline.
- Production priority is Startup/Rescan, configuration/DC, State Request,
  Mailbox, then Register Request. Domain and cyclic DC transport still run
  first. One accepted request remains `InFlight` across cycles and is never
  retransmitted merely because its response is delayed.
- Match the complete frozen Startup action and control request, including
  token, index, generation, address, operation, payload, response length,
  deadline, and WKC policy.
- Active or faulted rescan clears the topology lifecycle gate. Successful
  completion can restore only newly verified topology evidence; drive, DC,
  Domain, command-age, supervisor, safety, and motion-permit gates remain
  independent.
- Latch the first Startup/protocol error in the terminal result. Only another
  explicit `start_rescan` may retry from Faulted; ordinary WKC, timeout, link,
  or protocol faults never start one.

## 4. Validation & Error Matrix

| Condition | Result |
| --- | --- |
| Startup is Idle, Scanning, configuring, or transitioning | `InvalidState(phase)` |
| Another rescan is Active | `Busy` |
| Deadline is not strictly in the future | `InvalidDeadline` |
| Stale or replaced handle | `InvalidHandle` |
| Terminal result requested while Active | `ResultNotReady` |
| Retained plan/profile validation fails | `Startup(...)`, no evidence mutation |
| Overall deadline expires | first `Startup(OperationDeadlineExceeded)` fault |
| Scan/SII/register/AL/control request fails | first wrapped `Startup(...)` fault |
| A new valid explicit request follows a fault | new handle and fresh Startup reset |

## 5. Good / Base / Bad Cases

- Good: a Ready retained plan is submitted, old topology evidence disappears
  before the first APRD, the existing pipeline re-verifies every expected
  slave, and the result completes at PREOP.
- Base: the APRD probe response arrives one production cycle late. LRW and
  FRMW still run in each cycle, the shared pool retains one request, and no
  duplicate APRD is sent.
- Bad: infer a topology change from WKC and start rescan automatically, keep
  old verified records visible while probing, or use completion to return OP
  or restore motion permission.

## 6. Tests Required

- Startup unit tests: transactional invalid submission, stable/stale handles,
  explicit retry, retained-plan reuse, synchronous evidence clearing, complete
  pipeline deadline capping, PREOP-only policy, result reuse, and first fault.
- Scheduler tests: distinct Rescan selection, Startup-level priority, exact
  request matching, in-flight nonpreemption, terminal progress/fault, and no
  automatic trigger after completion or another fault.
- Linux integration: LRW/FRMW before APRD, delayed response across a cycle,
  no retransmission, one control slot, zero AL writes for the empty-topology
  case, and fail-closed/restored topology projection.
- Repository: focused tests, core no_std check, `make test-hil`, `make ci`,
  capability validation, diff checks, and exact pushed GitHub Actions success.

## 7. Wrong vs Correct

Wrong: clear and rebuild topology from an application loop, then infer that
the network may return to OP.

```rust
scan.start(...)?;
while !scan.done() { poll_and_retry(); }
topology_valid = true;
```

Correct: submit one bounded named operation to the retained Startup authority
and schedule its existing actions through the production service slot.

```rust
let handle = startup.start_rescan(generation, now_ns, deadline_ns)?;
let services = ScheduledProductionServices::new(Some(&mut startup), None, None, None);
let status = startup.rescan_status(handle)?;
```
