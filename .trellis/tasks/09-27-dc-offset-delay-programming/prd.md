# DC offset and propagation delay programming

## Goal

Close the next software boundary of PRD FR-018 / DC-002 by programming every
System-Time-capable EtherCAT slave with the propagation delay already derived
from the immutable DC topology and a corrected System Time Offset based on a
caller-supplied application-time sample.

The operation shall be bounded, allocation-free, exact-WKC controlled, and
published as complete only after every eligible slave has accepted one
combined offset/delay write. It shall be available through the unified
production scheduler and lifecycle configuration gate without changing the
existing SYNC0/SYNC1 controller contract.

## Background

- Online scan now classifies 32-bit and 64-bit DC clocks and Startup publishes
  a fixed-capacity topology with a selected reference and cumulative
  transmission delay per measurable DC slave.
- The existing `DcController` configures SYNC0/SYNC1 for one station, while
  `DcCyclicSync` injects application time into cyclic FRMW traffic. Neither
  owner currently initializes per-slave System Time Offset `0x0920` and System
  Time Delay `0x0928`.
- The ESC register contract requires offset and delay to remain coherent. The
  implementation therefore treats the 8-byte offset and 4-byte delay as one
  exact fixed-address write starting at `0x0920`.
- Hardware writes cannot be rolled back. Software evidence must therefore
  distinguish partial physical progress from a fully published configuration.

## Requirements

### R1. Deterministic topology-wide plan

1. A new fixed-capacity controller shall build its work plan only from a
   completed immutable `DcTopology` and shall process slaves in topology/scan
   order.
2. Only slaves classified as System-Time-capable shall be programmed. Delay-
   only and non-DC slaves remain observable in topology but shall emit no
   clock-programming action.
3. Every planned slave shall retain its scan position, station address, DC
   clock range, and cumulative transmission delay. Missing delay evidence or
   a missing selected reference shall reject start before any request is
   emitted.
4. The topology shall retain the scanned 32-bit/64-bit DC range so callers do
   not need a second mutable scan-record source during configuration.
5. Empty topologies and topologies without System-Time-capable slaves shall
   complete deterministically without fabricating requests or evidence.

### R2. Exact clock sample and offset calculation

1. For each planned slave the controller shall issue fixed-address read
   `0x0910` for exactly 24 bytes with expected WKC 1. The response supplies the
   current System Time and the previously programmed 64-bit offset.
2. The target application time shall be the start-time application sample plus
   checked elapsed monotonic time at response acceptance. Monotonic regression
   or application-time overflow shall fault with a typed error.
3. A 64-bit clock shall use a checked signed difference between target
   application time and sampled System Time. A difference that cannot fit the
   signed offset delta shall be rejected.
4. A 32-bit clock shall calculate the signed shortest delta from the low
   32-bit wrapping difference, preserving valid wrap-around behavior.
5. The new raw offset shall add the signed delta to the old raw offset with
   two's-complement wrapping semantics. No floating-point arithmetic or
   host-wall-clock access is permitted.

### R3. Atomic offset/delay register write

1. Each accepted read shall be followed by one fixed-address write beginning
   at `0x0920` with exactly 12 little-endian bytes: new 64-bit offset followed
   by unsigned 32-bit propagation delay.
2. The selected reference clock shall be written with transmission delay zero;
   every other planned slave shall use its validated topology delay.
3. The write shall require exact WKC 1, exact action ownership, current
   generation, and bounded request/operation deadlines.
4. A slave becomes locally completed only after its write succeeds. The next
   slave shall not begin before that terminal result is accepted.
5. Stale generations, wrong actions, malformed payloads, WKC mismatch,
   deadline expiration, and arithmetic failures shall latch a typed fault and
   stop further requests.

### R4. Publication and restart semantics

1. Immutable programmed-slave evidence shall include position, station,
   range, sampled system time, target application time, old/new offset, and
   programmed delay.
2. Public complete evidence shall remain empty until every planned write
   succeeds. Diagnostic progress may report a completed count without
   representing the operation as atomic or rolled back.
3. On fault, previously written ESC registers remain physical side effects,
   but the controller shall publish no complete batch and shall expose the
   exact fault plus bounded progress.
4. Restart after Idle, Complete, or Faulted shall clear prior evidence and
   rebuild the plan from the supplied topology and time samples.

### R5. Scheduler and lifecycle integration

1. The unified production scheduler shall expose an additive
   `DcClockConfiguration` service ahead of the existing single-station
   `DcConfiguration` SYNC service.
2. Existing scheduler construction shall remain source-compatible; callers
   opt in through a builder/attachment method and an explicit Startup
   configuration-service requirement.
3. When required, Startup shall not release into its prior identity/SII/AL
   path until the clock controller reaches Complete. Missing, faulted, or
   incomplete service state shall fail closed with typed scheduler/Startup
   evidence.
4. Lifecycle guard authorization shall classify both DC clock programming and
   existing DC SYNC programming as configuration work. No request may bypass
   the established generation and owner checks.

### R6. Tests and claim boundary

1. Unit tests shall cover multi-slave order, reference delay zero, non-zero
   downstream delay, 64-bit positive/negative correction, 32-bit wrap,
   existing-offset adjustment, elapsed-time correction, empty/no-clock
   topology, missing delay/reference, overflow/regression, WKC/payload/action/
   generation/timeout failures, restart clearing, and partial-write fault.
2. One public control/request integration test shall prove the 24-byte read and
   12-byte write traverse the production request/RX path with exact ownership
   and WKC policy.
3. Scheduler tests shall prove service ordering and Startup barrier release;
   lifecycle tests shall prove configuration-phase gating.
4. The core shall remain `no_std`, allocation-free, non-blocking, and free of
   logging, sleeps, and generic-RX register special cases.
5. README, requirements, backend specification, and capability manifest shall
   describe the implemented register programming while retaining explicit
   limits for clock-lock proof, application-time source authenticity, complete
   all-slave periodic synchronization, physical timing/HIL, WCET, ETG
   conformance, and functional safety.

## Acceptance Criteria

- [x] A topology-wide bounded controller emits exact `0x0910/24` reads and
      combined `0x0920/12` writes for every System-Time-capable slave in scan
      order and skips delay-only/non-DC slaves.
- [x] Tests prove 64-bit and 32-bit wrap-aware correction, old-offset
      adjustment, monotonic elapsed-time compensation, reference delay zero,
      and non-zero downstream delay.
- [x] Missing topology evidence and every ownership, shape, WKC, arithmetic,
      and timeout failure stop the sequence with typed evidence and no
      published complete batch.
- [x] Restart clears all prior evidence; successful completion exposes an
      immutable position-keyed programmed result for every planned slave.
- [x] The additive production service runs before existing DC SYNC
      configuration and participates in Startup/lifecycle configuration gates.
- [x] Public request/RX coverage proves 24-byte read and 12-byte write payloads
      end to end without adding register knowledge to generic control/RX code.
- [ ] Focused tests, no-std checks, repository `make ci`, `make bpf`, and GitHub
      Actions pass on the exact pushed commit.

## Out Of Scope

- Deriving an external application-time source, PTP/TAI discipline, or proving
  the authenticity/accuracy of the caller-supplied sample.
- Changing `AssignActivate`, calculating SYNC start time, configuring every
  slave's SYNC0/SYNC1 units, or proving all-slave runtime lock.
- ESI/SII DC category parsing, redundant-ring delay correction, hot-connect
  topology changes, or online recalculation after Startup.
- Physical cable/device precision, target WCET, long-duration HIL, ETG
  conformance, or functional-safety qualification.

## Notes

- This task deliberately adds a separate topology-wide clock-alignment owner;
  the established one-station `DcController` remains the SYNC pulse owner.
- The software publication is transactional, but accepted ESC writes are not
  reversible. Fault evidence must never describe partial hardware work as a
  rollback.
