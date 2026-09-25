# eBPF Linux raw port stall attribution

## Goal

Add bounded Linux runtime evidence for unexpectedly slow `LinuxRawPort`
`send(2)` and nonblocking `recv(2)` calls so an operator can distinguish an
ESOP raw-port syscall stall from an application gateway stall or an
uncorrelated EtherCAT cycle failure.

The result must remain an observation mechanism. It must not block the hard
realtime path, write motion state, or claim that a syscall-duration sample
proves driver, NIC, slave, or complete-cycle latency.

## Background

- `LinuxRawPort::tx_submit` validates the frame and then calls `libc::send`;
  `rx_poll` calls nonblocking `libc::recv` and maps frame, empty, link-down,
  and error outcomes (`crates/esop-ethercat-linux-port/src/lib.rs:119`,
  `crates/esop-ethercat-linux-port/src/lib.rs:146`).
- The eBPF bundle already supports stable versioned user markers, transactional
  begin/end uprobe attachment, policy epochs, fixed-capacity in-flight state,
  fixed 96-byte evidence, and cycle-risk correlation for Zenoh gateway stalls
  (`bpf/esop_runtime.bpf.c`, `crates/esop-ebpf-runtime/src/linux.rs`).
- The runtime-observability design still lists Linux RT port send and receive
  boundaries as an open user-space observation surface
  (`docs/esop-ebpf-runtime-observability.md:148`).
- The Linux raw port remains a development/HIL capability without production
  realtime qualification (`capability_manifest.json`, capability
  `linux_raw_port`).

## Requirements

### R1. Stable raw-port marker ABI

- Export exact versioned C ABI symbols from `esop-ethercat-linux-port`:
  `esop_linux_raw_port_operation_begin_v1(ifindex, operation)` and
  `esop_linux_raw_port_operation_end_v1(ifindex, operation, outcome)`.
- Use bounded integer operation and outcome codes. TX and RX must be
  distinguishable; TX success/error/partial and RX frame/empty/link-down/error
  must remain distinguishable.
- Markers must not allocate, parse strings, lock, sleep, read a clock, or wait
  for the observer. With no attached uprobe they are fixed no-op calls.
- TX markers bracket only a valid frame's `send(2)` call. RX markers bracket
  only the `recv(2)` call. The terminal marker must be emitted exactly once for
  every begun operation, including error and partial/empty outcomes.
- Simulator and `esop-ethercat-core` behavior must remain unchanged.

### R2. Bounded eBPF duration evidence

- Add an explicit raw-port stall threshold and nonzero policy epoch to the
  kernel context ABI. A zero threshold is invalid.
- Track at most 1024 in-flight raw-port operations in an LRU map keyed by the
  current process/thread identity. A new begin on an occupied key must recover
  by replacing stale state and accounting for a mismatch.
- End processing must remove matched state before validating operation,
  interface, outcome, epoch, or threshold emission conditions.
- Emit one fixed 96-byte `UserEsop/RawPortStall` record only when
  `duration_ns > raw_port_stall_threshold_ns`.
- Evidence must carry TGID/TID, CPU, interface index, duration, threshold,
  operation/outcome detail, and a stable nonzero event identity derived from
  the begin sample. It must not claim driver queue, NIC DMA, wire, slave, or
  full-cycle duration.
- Add saturating per-CPU counters for begins, completions, stalls, and
  mismatches. Map ABI size changes must be explicit and identical in C and
  Rust.

### R3. Runtime configuration and atomic attachment

- Add `raw_port_stall_threshold_ns` to `RuntimeConfig`, `KernelContext`, and an
  atomic `update_raw_port_tracking` API that advances the raw-port epoch.
- A tracked-PID change must also advance the raw-port epoch so old state cannot
  be attributed under a new process policy.
- Reserve a disjoint begin/end attach-mask pair and expose
  `attach_raw_port_probes(target, pid, required)`.
- The pair must attach transactionally: second-probe failure detaches the first
  and publishes neither capability bit. Optional failure preserves existing
  capabilities; required failure returns the typed loader error.
- Nonpositive explicit PIDs and incomplete/overlapping pair specifications are
  invalid configuration.

### R4. Incident classification

- Add `EvidenceKind::RawPortStall` and `IncidentCode::HostPortStall` without
  changing the 96-byte runtime evidence layout or protobuf field layout.
- Classification requires all of: nonzero threshold, strict over-threshold
  duration, `observed_value == duration_ns`, and a correlated cycle carrying
  deadline, WKC, or DC risk.
- A valid event produces an Error-level controlled-stop recommendation with a
  confidence distinct from gateway stalls. At/below-threshold, inconsistent,
  zero-threshold, and healthy-cycle events produce no incident.
- eBPF evidence remains advisory and cannot directly write controlword,
  lifecycle state, or motion permit.

### R5. Evidence-bound documentation

- Update the runtime-observability design, capability manifest, README/status
  claim where applicable, and Trellis quality contract to describe the exact
  implemented boundary and remaining target qualification gaps.
- Keep target-kernel uprobe attachment, injected syscall delay, overhead/WCET,
  driver/NIC attribution, real slave behavior, and production qualification
  explicitly open.

## Acceptance Criteria

- [x] AC1: The Linux raw-port crate exports and exact-links both versioned
  marker symbols; pure tests cover all bounded operation/outcome encodings.
- [x] AC2: Valid TX and every RX call bracket the underlying syscall without
  adding allocation, locking, timing, logging, or observer waits to the port.
- [x] AC3: The BPF source contains a fixed-capacity raw-port operation map,
  strict threshold comparison, state deletion, epoch/interface/operation/
  outcome validation, fixed detail encoding, and four explicit counters.
- [x] AC4: Rust and C agree on context/stat ABI sizes; the runtime evidence ABI
  remains 96 bytes and decodes `UserEsop/RawPortStall` correctly.
- [x] AC5: Runtime configuration rejects zero threshold without mutation,
  advances epochs on valid threshold/tracked-PID changes, and reports
  saturating raw-port statistics.
- [x] AC6: Raw-port uprobe begin/end bits are disjoint from kernel and Zenoh
  bits, and the public pair attach API uses transactional rollback semantics.
- [x] AC7: Correlator tests accept one internally consistent over-threshold
  risk-cycle raw-port stall and reject at-threshold, inconsistent,
  zero-threshold, and healthy-cycle variants.
- [x] AC8: Focused crate tests, workspace format/lint/tests, BPF syntax checks,
  and a real CO-RE object build pass.
- [x] AC9: Documentation and capability claims state the implemented syscall
  boundary and do not imply production realtime, driver/NIC, wire, slave, or
  full-cycle qualification.

## Out Of Scope

- Kernel driver queue, NAPI, IRQ-to-packet, NIC DMA, wire, slave-response, WKC,
  or complete EtherCAT cycle latency measurement.
- Runtime delay injection or production overhead qualification on target
  hardware.
- Changes to simulator semantics, EtherCAT core traits, MLG authority,
  protobuf wire fields, ROS 2 hooks, or non-Linux targets.
