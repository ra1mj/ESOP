# SOEM parity and runtime diagnostics roadmap

## Goal

Evolve ESOP toward the practical master capabilities exposed by SOEM 2.x while
preserving ESOP's existing allocation-free DC, fixed scheduling, exact WKC,
and fail-closed lifecycle contracts. Provide one stable operator command
surface that can inspect the resulting runtime and correlate EtherCAT faults
with host eBPF evidence without touching the real-time cycle.

## Background

- ESOP already implements DC capability discovery, topology and propagation
  delay evidence, clock programming, SYNC0/SYNC1 programming, cyclic reference
  synchronization, synchronization-window monitoring, and lifecycle gating.
- The remaining SOEM-class gaps are primarily optional mailbox protocols,
  cable redundancy, product-qualified automatic recovery, hardware ports, and
  real-device interoperability rather than basic DC support.
- Runtime observations already exist in EtherCAT snapshots, ProcBuf v7,
  Protobuf, Zenoh, diagnostic rings, and the eBPF incident correlator, but no
  unified operator CLI owns their presentation or compatibility contract.

## Requirements

### R1. Preserve DC as a release invariant

Every new master capability shall coexist with the current DC reference-clock,
SYNC0/SYNC1, cyclic FRMW, synchronization-window, and fail-closed lock
qualification paths. Optional protocol or diagnostic load must not mutate the
frozen P0 frame plan or bypass exact-generation/WKC checks.

### R2. Deliver SOEM parity as bounded independent capabilities

Mailbox cyclic handling, FoE, EoE, SoE, redundancy, and supervised recovery
shall be planned and implemented as independent capabilities with compile-time
or product-policy admission. They shall reuse the existing control request
pool and production scheduler instead of adding a second transport owner.

### R3. Maintain one stable operator command namespace

The repository shall own an `esop` command namespace for read-only status,
topology, Domain/WKC, DC, lifecycle, runtime incident, watch, and doctor
operations. Command names and typed payload semantics shall be versioned and
documented independently from the selected local or remote transport.

### R4. Correlate eBPF without making it a safety dependency

The command surface shall expose eBPF agent health, event loss, and correlated
runtime incidents beside EtherCAT cycle facts. Loading, polling, rendering, or
failure of eBPF and the CLI must never block the EtherCAT cycle or grant motion
authority.

### R5. Evidence-bound claims

Software tests, simulator evidence, Linux host qualification, real-device HIL,
target WCET, and functional-safety qualification shall remain distinct claims.
The roadmap must not describe planned SOEM parity as implemented support.

## Child Deliverables

1. `09-28-runtime-diagnostics-cli-ebpf`: versioned runtime status projection,
   ROS2-like read-only commands, incremental watch, and eBPF incident display.
2. `09-28-soem-dc-parity`: ordered capability plan and implementation slices
   for missing SOEM-class behavior while retaining DC invariants.

## Acceptance Criteria

- [ ] Both child tasks have testable PRDs, designs, implementation plans, and
  explicit qualification boundaries.
- [ ] The diagnostic CLI consumes typed host-domain snapshots and incidents;
  it has no dependency path into the activated EtherCAT cycle.
- [ ] The SOEM parity plan names the order, scheduler ownership, DC interaction,
  feature gate, and validation boundary for each missing capability.
- [ ] Documentation and the capability manifest distinguish implemented,
  partial, development/HIL, and planned work.
- [ ] Each completed child passes `make ci` and is committed independently.

## Out of Scope

- Claiming functional-safety certification, STO, FSoE, or safe motion from CLI
  or eBPF evidence.
- Treating a software simulator or AF_PACKET test as target hardware
  qualification.
- Adding unbounded discovery, logging, allocation, or asynchronous callbacks to
  the real-time cycle.
