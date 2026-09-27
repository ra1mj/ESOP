# Design

## Architecture

Add `register_request.rs` to `esop-ethercat-core` with a caller-owned
`EscRegisterRequestController<const CAPACITY: usize>`. The controller owns a
fixed slot array and a fixed FIFO of slot indexes. Terminal slots retain their
result until explicit release, while the FIFO contains only queued or active
work.

`EscRegisterRequestHandle` carries `{ slot, generation }`. Slot generation is
advanced on each reuse, making state/data/release lookups fail closed for stale
handles. Each slot retains the fixed station address, register offset,
operation, absolute deadline, actual WKC, typed terminal error and a bounded
`MAX_CONTROL_PAYLOAD` byte buffer.

## Data Flow

1. The application submits a read or write and receives a stable handle.
2. The production scheduler selects the register service only when all higher
   priority services are inactive.
3. The controller freezes the FIFO head into an exact `EscRegisterAction` and
   marks the request `Busy`.
4. `enqueue_pending` allocates the existing shared `ControlRequestPool` entry.
5. The scheduled bank submits DC first and at most one prepared control frame,
   then performs the common bounded Domain/DC/control RX finalizer.
6. On terminal pool state, the controller verifies action ownership, copies a
   read response, records WKC/error, removes the FIFO head and releases the pool
   handle.
7. The application polls state/data and explicitly releases the terminal slot.

## Contracts And Invariants

- Fixed-station `FPRD`/`FPWR` only; addresses use the shared `fixed_address`
  helper.
- Request length is `1..=MAX_CONTROL_PAYLOAD` and register plus length must fit
  the 16-bit ESC register space.
- One controller-wide datagram index is reused serially; request generation and
  the RX index table reject late responses from earlier uses.
- One frozen action owns one pool request. Scheduler ownership is checked with
  `ControlRequest::matches_action` before TX and again on completion.
- Queue mutation is transactional: invalid submission does not consume a slot
  or FIFO entry; terminal completion pops exactly the FIFO head.
- A terminal slot is not silently reused. Explicit release is required.
- Request failures are local results. They are reported for the affected cycle
  but do not permanently fault the controller or prevent later queued work.
- The service is appended after Mailbox in the scheduler's fixed-priority list.
  Existing service selection and startup configuration barriers are unchanged.

## Compatibility

Add a trailing defaulted const generic to `ScheduledProductionServices` for
register request capacity and an optional `with_register_requests` builder.
Existing generic instantiations keep compiling because the new parameter has a
default and the service remains absent unless explicitly bound.

No wire format, ProcBuf ABI, generated product schema or lifecycle state layout
changes are required.

## Failure Handling

- Submission validation errors return before queue mutation.
- An expired FIFO head becomes request-local `Error(Timeout)` and is removed
  from execution order.
- Pool terminal failures retain the precise `ControlError` inside the request
  error.
- Action mismatch fails the owning request, releases the pool slot and prevents
  response publication.
- If cyclic/DC submission prevents a prepared register frame from reaching the
  wire, the scheduler releases only the pool entry. The frozen register action
  remains rebuildable until its absolute deadline.

## Evidence Boundary

Unit and simulator tests establish deterministic software ownership, ordering,
validation and bounded scheduling. They do not prove that a physical response
came from the intended ESC or establish real-device timing and interoperability.
