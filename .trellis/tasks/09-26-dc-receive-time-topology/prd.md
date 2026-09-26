# DC receive-time topology and propagation delay

## Goal

Close the next software boundary of PRD FR-018 / DC-001 by extending online
scan with exact EtherCAT Data Link port-state and Distributed Clocks receive-
time evidence, deriving the physical slave tree from scan order, and computing
bounded propagation-delay evidence relative to Startup's selected reference
clock before identity, SII, or AL progression.

The result is an immutable, position-keyed topology projection for later DC
offset/delay programming. It is not yet clock configuration or hardware timing
qualification.

## Background

- The previous task now identifies base DC support, distinguishes System-Time
  capable and delay-only ESCs, and selects an explicit or fallback reference
  clock.
- The scanner retains the ESC port descriptor but does not read Data Link
  Status `0x0110`, so it cannot distinguish forwarding/open ports from closed
  loops and cannot reconstruct branches.
- DC-capable ESCs expose four 32-bit receive timestamps at `0x0900..0x090F`.
  These timestamps and the physical tree are the inputs to the established
  round-trip/one-way propagation-delay calculation.
- Existing `DcController`, `DcCyclicSync`, and `DcMonitor` already configure
  SYNC and monitor clock quality, but they do not own topology discovery and
  must not infer delay values from caller-selected stations.

## Requirements

### R1. Exact port-state and receive-time scan evidence

1. Every discovered slave shall be read at fixed address `0x0110` for the
   exact two-byte Data Link Status register before its scan record is
   published.
2. The scanner shall decode, retain, and expose per-port `link_up`,
   `loop_closed`, and `signal_detected` bits for all four ESC ports while
   preserving the raw register value.
3. Every base-DC-capable slave, including a System-Time-probe WKC-0 delay-only
   slave, shall be read at fixed address `0x0900` for exactly 16 bytes and
   retain four little-endian 32-bit receive timestamps.
4. Data Link Status and receive-time reads require exact WKC 1 and exact
   payload length. Invalid WKC, malformed payload, stale generation,
   ownership mismatch, or timeout shall fault scan without publishing the
   current partial record.
5. Non-DC slaves shall skip the receive-time read but shall still publish Data
   Link Status evidence and follow the existing configuration/status path.

### R2. Deterministic physical topology projection

1. After the complete scan, the system shall derive one bounded physical tree
   from scan/ring order and per-port loop state. Port 0 is the upstream edge;
   downstream ports are consumed in EtherCAT traversal order 3, 1, 2.
2. Each projected port shall expose the adjacent physical slave position, if
   present. The root port 0 and closed/unconnected downstream ports shall have
   no adjacent position.
3. A scan containing more open downstream edges than records, records that
   cannot be reached from the first slave, duplicate positions, or capacity
   violations shall return a typed topology error and publish no topology.
4. The projection shall remain allocation-free, fixed-capacity, deterministic,
   and independent of transport scheduling.

### R3. DC propagation-delay calculation

1. For each DC-capable slave and connected downstream branch, the system shall
   find the first downstream DC-capable slave and compute the established
   one-way link delay from source-port round-trip time minus that downstream
   DC slave's internal connected-port round-trip sum, divided by two.
2. Receive-time subtraction shall use 32-bit timestamp wrap semantics. Aggregate
   sums, downstream subtraction, and cumulative reference delays shall use
   checked arithmetic and return typed errors on impossible underflow or
   overflow.
3. Both ends of every measurable DC-to-DC link shall expose the peer DC
   position and one-way propagation delay.
4. When a valid reference clock is selected, the projection shall assign that
   slave cumulative transmission delay zero and compute cumulative delay for
   all DC slaves reachable through measurable DC links, regardless of whether
   the explicit reference is the first DC slave.
5. DC-capable slaves that cannot be connected to the selected reference by
   measurable links shall retain `None` delay evidence. If such a slave is
   declared `SystemTime` or `ReferenceClock` required by the product profile,
   Startup shall fail closed before identity/SII/AL actions.
6. A topology with no selected reference remains valid when no profile
   requires DC; physical topology and available link evidence shall still be
   observable without inventing cumulative delay.

### R4. Startup publication and lifecycle semantics

1. Startup shall construct and validate the topology only after scan count and
   DC policy validation have succeeded, and before any identity read.
2. Reference selection and topology publication shall be transactional: any
   validation or calculation error leaves both unpublished and latches a typed
   Startup fault.
3. Startup shall expose immutable topology records, selected reference
   position/station, per-port evidence, measurable link delays, and per-slave
   cumulative transmission delay.
4. Every restart shall immediately clear selected-reference and topology
   evidence before new scan actions are emitted.
5. Existing products with no DC requirements and non-DC topologies shall
   continue to start through the prior bounded path.

### R5. Documentation, tests, and claim boundary

1. Unit tests shall cover linear, branched, delay-only, mixed DC/non-DC,
   explicit non-first reference, timestamp wrap, malformed topology,
   arithmetic failure, WKC/payload/timeout failure, required unreachable DC,
   and restart clearing.
2. One public master/control integration test shall prove a 16-byte receive-
   time response and exact WKC policy traverse the production request/RX path.
3. The core shall continue to build as `no_std` and add no allocation, wait,
   sleep, logging, or address-specific behavior to the generic RX engine.
4. README, PRD/requirements, backend specification, and capability manifest
   shall describe the implemented evidence and retain explicit limitations for
   physical authenticity/precision, offset/delay writes, application time,
   SYNC configuration, all-slave synchronization, WCET, HIL, conformance, and
   functional safety.

## Acceptance Criteria

- [x] Scan emits exact `0x0900/16` receive-time and `0x0110/2` Data Link
      Status actions in the correct DC/non-DC sequences and publishes decoded
      evidence only after exact WKC/payload validation.
- [x] Delay-only DC slaves still publish receive times, while non-DC slaves
      skip only the receive-time action.
- [x] Fixed-capacity topology tests prove linear and branched port adjacency
      in 3, 1, 2 traversal order and reject overrun, unreachable, duplicate,
      and capacity-invalid inputs transactionally.
- [x] Propagation tests prove measurable DC-link delay, timestamp wrap,
      checked underflow/overflow rejection, non-first reference traversal,
      mixed DC/non-DC handling, and explicit `None` for unmeasurable delay.
- [x] Startup publishes topology before identity only after complete policy
      validation, rejects required unreachable DC, and clears all evidence on
      restart or fault.
- [x] Public request/RX integration coverage proves the new fixed-address
      reads retain exact working-counter ownership end to end.
- [x] Focused core tests, no-std checks, repository `make ci`, `make bpf`, and
      GitHub Actions pass on the exact pushed commit.

## Out Of Scope

- Writing ESC System Time Offset `0x0920` or System Time Delay `0x0928`.
- Application-time injection, start-time calculation, SYNC0/SYNC1 activation,
  periodic all-slave synchronization, or motion gating on measured clock lock.
- ESI/SII DC category parsing or automatic `AssignActivate` generation.
- Physical response provenance, measured cable/device precision, target WCET,
  long-duration HIL, ETG conformance, or functional-safety qualification.

## Notes

- This task extends the existing scan/Startup owner; it does not add a second
  control-plane request scheduler.
- Optional, unmeasurable DC evidence remains visible as incomplete instead of
  being silently converted to zero. Product-required DC remains fail-closed.
