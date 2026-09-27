# FMMU register descriptor discovery

## Goal

Close the software-only FMMU register-discovery gap by reading every ESC-reported
FMMU register page before the first AL transition, publishing the pages only as
part of the same successful per-slave SII verification transaction, and allowing
the mapping service to clear and verify the complete discovered FMMU bank before
programming the master-owned logical mapping.

## Background

- Online scan already retains the exact ESC-reported FMMU count from register
  `0x0004` in each `ScanRecord`.
- Startup already verifies ordered SII FMMU usage and rejects a usage count that
  exceeds the ESC-reported count, but it does not read the FMMU register pages.
- `MappingConfigController` currently writes and reads back only the configured
  FMMUs. Unused ESC slots can therefore retain stale state.
- FMMU register pages have the standard 16-byte layout at `0x0600 + index * 16`.
- The project IgH baseline clears the complete ESC-reported FMMU register space
  before applying configured FMMUs. ESOP must implement its own bounded,
  caller-driven state machines and must not copy third-party code.

## Requirements

### R1. Fixed-capacity register evidence

The no-std core shall expose a fixed-capacity representation of one decoded
16-byte FMMU register descriptor and one position/station-bound FMMU register
bank. The bank shall retain the exact descriptor order and reported count, with
no allocation and a maximum of 16 descriptors.

### R2. Bounded discovery controller

A caller-driven controller shall read every descriptor from index zero through
`reported_count - 1` using fixed-address reads, exact 16-byte responses, WKC 1,
one absolute discovery deadline, bounded per-request deadlines, generation and
action ownership checks, and the existing control-request pool contract.

Zero-count banks shall complete without emitting a register action. Counts above
the fixed capacity, address overflow, malformed response shape, WKC mismatch,
timeout, stale generation, substituted action, or control-pool failure shall
latch a typed first failure and publish no bank.

### R3. Startup integration and atomic publication

For profiles that require live SII structural verification, Startup shall run
FMMU register discovery after identity and optional mailbox verification and
before the SII configuration stream and first AL action. The controller shall
use the station address and FMMU count retained by the completed scan record.

Startup shall expose position-keyed verified FMMU register evidence only after
the matching SII signature/DC descriptor transaction also succeeds. A register
or SII failure shall publish no FMMU, SII, or DC evidence for that slave. Restart
shall clear all staged and published evidence.

Legacy profiles without an expected SII signature shall preserve their existing
identity/mailbox/DC/AL path and shall not gain implicit register traffic.

### R4. Complete-bank clearing and mapping qualification

`MappingConfigController` shall provide an enhanced start path bound to a
verified FMMU bank. Before writing SyncManager or configured FMMU entries it
shall write a zero descriptor to every discovered slot, read each slot back
exactly, and only then continue with the existing mapping sequence.

The enhanced path shall reject a station mismatch, an unverified/over-capacity
bank, or a configured FMMU count/index outside the discovered bank before
emitting any action. Clear or configured readback mismatch shall latch the
mapping fault. Existing `start` behavior remains source-compatible for callers
that have not opted into verified discovery.

Startup shall provide a typed bridge that starts mapping for a position only
from the matching published bank, so production callers do not reconstruct or
trust an independent count.

### R5. Documentation and capability boundaries

README, software PRD, capability manifest, and backend product-configuration
spec shall describe the implemented software evidence and full-bank clearing.
They shall continue to state that logical addresses are master-owned and that
physical response authenticity, target timing/WCET, interoperability,
long-duration operation, and hardware HIL remain unqualified.

## Acceptance Criteria

- [x] A register-discovery unit test proves exact addresses, count/order,
      decode, control-pool ownership, zero-count completion, and complete-only
      publication.
- [x] Negative tests cover over-capacity count, wrong WKC, short payload, stale
      generation/action, timeout, and restart/reset behavior without partial
      evidence.
- [x] Startup tests prove that an expected-SII profile emits descriptor reads
      before SII category-stream work and AL, publishes FMMU/SII/DC evidence
      atomically, and clears all evidence on restart or terminal failure.
- [x] Mapping tests prove that the enhanced path clears and reads back all
      discovered slots, including unused slots, before configuring used slots;
      station/count/index mismatch and clear readback mismatch fail closed.
- [x] A public integration path proves the enhanced mapping controller remains
      compatible with the production service scheduler/control pool.
- [x] `cargo test -p esop-ethercat-core --no-fail-fast` passes.
- [x] `cargo test -p esop-ethercat-linux-port --test scheduled_domains` passes.
- [x] `make ci` passes.
- [ ] The final commit is pushed to `ra1mj/ESOP` and the exact pushed SHA has a
      successful GitHub Actions `quality` run.

Local acceptance evidence was collected on 2026-09-27. The focused core and
Linux integration suites passed, followed by the repository-wide `make ci`
gate covering formatting, diff checks, workspace all-feature tests, Clippy with
warnings denied, release and `aarch64-unknown-none` builds, BPF C syntax, and
the generated configuration/qualification validators.

## Out of Scope

- Taking logical-address ownership away from the master-generated Domain plan.
- Trusting discovered pre-existing descriptors as the desired product mapping.
- Authenticating that a response came from the intended physical ESC.
- EtherCAT conformance, target-platform timing/WCET, or physical HIL claims.
- Vendor-specific FMMU register extensions beyond the standard 16-byte page.
