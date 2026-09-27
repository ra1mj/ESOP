# EtherCAT asynchronous ESC register requests

## Goal

Close the software-verifiable portion of EtherCAT requirement `REG-001` with
an application-facing, allocation-free asynchronous ESC register request API.
The API must support fixed-station reads and writes while preserving cyclic
PDO/DC priority and the existing bounded production receive window.

## Background

- `ControlRequestPool` already provides exact command, generation, address,
  length, deadline and WKC matching for one register datagram.
- `ScheduledProductionServiceScheduler` already admits at most one control
  request per cycle after the caller has submitted due process Domain frames,
  and retains in-flight ownership across cycles.
- Internal startup/configuration controllers use that machinery, but there is
  no reusable application-facing register request object comparable to IgH's
  `reg_request` API.
- `docs/ethercat-master-requirements.md` marks `REG-001` as P1 and requires
  asynchronous register reads/writes that do not defer the specified PDO
  cycle.

## Requirements

1. Provide a `no_std`, fixed-capacity controller that accepts multiple queued
   fixed-station ESC register reads and writes without heap allocation,
   blocking, sleeping or busy waiting.
2. Return stable handles containing a slot and generation so stale handles
   cannot observe or release a reused request.
3. Expose explicit `Free`, `Queued`, `Busy`, `Success` and `Error` request
   states plus operation metadata, actual WKC, response bytes and typed errors.
4. Validate station address, nonzero bounded length, 16-bit register range,
   timeout arithmetic, capacity and terminal-only release before mutating the
   queue.
5. Preserve FIFO ordering. A terminal request leaves the execution queue but
   keeps its result until the application explicitly releases its handle.
6. Bind each active action to the shared `ControlRequestPool`; accept only the
   exact operation, datagram index, generation, packed address, length,
   deadline and WKC expected by the request.
7. Integrate the controller into `ScheduledProductionServiceScheduler` as the
   lowest-priority service after mailbox. Existing startup, configuration, DC
   and mailbox transactions must not be overtaken.
8. Submit at most one register control datagram in a production service cycle.
   Due process Domain traffic and the DC sample retain their existing order and
   deadlines; an in-flight register request is never retransmitted.
9. A prepared request released because an earlier cyclic/DC stage failed must
   remain rebuildable until its own absolute deadline. Timeout, TX failure,
   malformed/mismatched completion and WKC failure must become request-local
   terminal errors and free scheduler ownership for later requests.
10. Update the requirement narrative and evidence-bound capability manifest.
    Software simulation must not claim physical ESC response authenticity,
    interoperability, target WCET, HIL, ETG conformance or functional safety.

## Acceptance Criteria

- [x] Public unit tests cover read/write success, FIFO ordering, response data,
      capacity, invalid shape/range/deadline, stale handles and terminal release.
- [x] Fault tests cover timeout, control-pool failure and action ownership
      mismatch without publishing partial success or leaking the pool slot.
- [x] Production scheduler tests prove register requests are selected only
      after higher-priority services and are executed through the shared
      Domain/DC/control RX finalizer.
- [x] A Linux simulated production cycle proves process traffic remains first,
      only one register control request is admitted, and the result is consumed
      on a later bounded cycle when necessary.
- [x] Existing callers compile without source changes through trailing generic
      defaults and the optional service binding.
- [x] `cargo test -p esop-ethercat-core --all-features` and the focused Linux
      port tests pass.
- [x] `make test-hil` and `make ci` pass.
- [x] `capability_manifest.json` and
      `docs/ethercat-master-requirements.md` describe the implemented software
      boundary and retain the hardware qualification limitations.

## Out Of Scope

- Auto-increment addressing, broadcast register requests or arbitrary logical
  memory access.
- Cancellation of an in-flight EtherCAT datagram.
- Parallel register transactions in one production cycle.
- MCU port work, real-slave interoperability, timing/WCET qualification, HIL,
  ETG conformance or safety certification.
