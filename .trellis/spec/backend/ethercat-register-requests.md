# EtherCAT Asynchronous Register Requests

## 1. Scope / Trigger

Use this contract when application or diagnostic code needs asynchronous ESC
register access during production operation. It applies only to fixed-station
`FPRD` and `FPWR` requests routed through the shared production service
scheduler. Startup/configuration controllers keep their dedicated state
machines.

## 2. Signatures

```rust
EscRegisterRequestController::<CAPACITY>::new(datagram_index)
controller.submit_read(station, register, length, now_ns, timeout_ns)
controller.submit_write(station, register, data, now_ns, timeout_ns)
controller.status(handle)
controller.data(handle)
controller.release(handle)
services.with_register_requests(&mut controller)
```

`submit_*` returns an `EscRegisterRequestHandle` containing a slot and
generation. `status` returns the state, operation, fixed address components,
length, absolute deadline, actual WKC and optional typed error.

## 3. Contracts

- `CAPACITY` is compile-time fixed and must be in `1..=64` before submission.
- Submission is transactional: invalid input does not reserve a slot or alter
  FIFO order.
- States progress `Free -> Queued -> Busy -> Success|Error`; only terminal
  handles may be released.
- Terminal data and error evidence remain caller-owned until `release`.
- The scheduler selects register requests after startup, PDO configuration,
  watchdog, mapping, DC and mailbox services.
- At most one register control datagram is admitted per production cycle.
  Prepared requests may be rebuilt before their absolute deadline; in-flight
  requests retain ownership and are never retransmitted.
- The completion must match operation, datagram index, generation, packed
  fixed address, length, deadline and WKC through `ControlRequestPool`.
- Register failures are request-local. They do not become a CoE/lifecycle gate
  unless the product layer explicitly elevates the diagnostic result.

## 4. Validation & Error Matrix

| Condition | Result |
| --- | --- |
| Capacity is zero or greater than 64 | `InvalidCapacity` |
| Fixed station address is zero | `InvalidStationAddress` |
| Length is zero or exceeds `MAX_CONTROL_PAYLOAD` | `InvalidLength` |
| Register plus length exceeds the 16-bit ESC range | `RegisterRangeOverflow` |
| Timeout is zero or absolute deadline overflows | `InvalidTimeout` |
| No free request slot | `CapacityExceeded` |
| Slot/generation is stale or absent | `InvalidHandle` |
| Release before terminal state | `InvalidState` |
| Shared control pool rejects admission | terminal `Control { request, error }` |
| Frozen action differs from terminal pool request | terminal `ActionMismatch` |
| Absolute deadline is reached before/after completion | terminal `Timeout` |
| Exact WKC check fails | terminal `Control` retaining `actual_wkc` |

## 5. Good / Base / Bad Cases

- Good: queue a read and write, observe FIFO `Busy`/terminal states, consume
  bounded data, then explicitly release both handles.
- Base: a cyclic/DC stage releases a prepared pool request; the same frozen
  register action is rebuilt on a later cycle before its deadline.
- Bad: retry or retransmit an in-flight request, bypass generation/WKC checks,
  or turn an auxiliary register failure into a motion gate implicitly.

## 6. Tests Required

- Unit: read/write success, FIFO ordering, stale handles, capacity, range,
  timeout arithmetic, terminal release and actual WKC retention.
- Unit: action mismatch and occupied control pool end only the owning register
  request and do not release another pool occupant.
- Scheduler: mailbox remains higher priority and control-pool admission failure
  is returned as `ScheduledProductionServiceFault::RegisterRequest`.
- Linux integration: assert `LRW -> FRMW -> FPRD`, hold the register response
  across a cycle boundary, assert no second `FPRD`, then consume the result.
- Repository: `make test-hil` and `make ci` must pass.

## 7. Wrong vs Correct

Wrong: acquire directly from the shared pool and treat failure as a global
cycle error.

```rust
let handle = controls.acquire(index, generation, address, op, data, deadline)?;
```

Correct: bind the fixed controller to the production scheduler so admission,
ownership and request-local failure are handled together.

```rust
let request = registers.submit_read(station, register, len, now_ns, timeout_ns)?;
let services = ScheduledProductionServices::new(None, None, None, None)
    .with_register_requests(&mut registers);
```
