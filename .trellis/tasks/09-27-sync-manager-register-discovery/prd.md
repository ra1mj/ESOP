# SyncManager register bank discovery

## Goal

Close the software-only SyncManager register-discovery gap by reading every
ESC-reported standard SyncManager register page before the first AL transition,
publishing the pages only as part of the same successful per-slave SII
verification transaction, and allowing the mapping service to clear and verify
the complete discovered SyncManager bank before programming the product mapping.

## Background

- Online scan already retains the exact ESC-reported SyncManager count from the
  12-byte basic ESC information block in each `ScanRecord`.
- Startup already verifies the SII SyncManager/PDO structural signature, but it
  does not read live SyncManager register pages or bind them to that evidence.
- `MappingConfigController` writes and reads back only configured SyncManagers.
  Unused ESC slots can therefore retain stale activation, control, status, or
  address state across configuration attempts.
- Standard SyncManager pages have an eight-byte layout at
  `0x0800 + index * 8`, and the core already has a fixed maximum of 16
  SyncManagers.
- The completed FMMU discovery path establishes the project pattern for bounded
  discovery, atomic Startup publication, typed mapping qualification, and
  complete-bank reset without treating observed state as desired configuration.

## Requirements

### R1. Fixed-capacity register evidence

The `no_std` core shall expose a fixed-capacity representation of one exact
eight-byte SyncManager register descriptor and one position/station-bound
SyncManager register bank. The bank shall retain descriptor order and the exact
ESC-reported count with no allocation and a maximum of 16 descriptors.

The descriptor shall preserve raw bytes and expose the standard physical start,
length, control, status, activation, and PDI control fields. Disabled or zeroed
pages remain valid observations.

### R2. Bounded discovery controller

A caller-driven controller shall read every descriptor from index zero through
`reported_count - 1` using fixed-address reads, exact eight-byte responses,
WKC 1, one absolute discovery deadline, bounded per-request deadlines,
generation/action ownership checks, and the existing control-request pool
contract.

Zero-count banks shall complete without a register action. Counts above the
fixed capacity, address overflow, malformed response shape, WKC mismatch,
timeout, stale generation, substituted action, or control-pool failure shall
latch a typed first failure and publish no bank.

### R3. Startup integration and atomic publication

For profiles that require live SII structural verification, Startup shall run
SyncManager register discovery after the existing FMMU register discovery and
before the SII category stream and first AL action. The controller shall use the
station address and SyncManager count retained by the completed scan record.

Startup shall expose position-keyed verified SyncManager register evidence only
after the matching FMMU bank, SII signature, and optional DC descriptor
transaction succeeds. A register or SII failure shall publish no FMMU,
SyncManager, SII, or DC evidence for that slave. Restart shall clear all staged
and published evidence.

Legacy profiles without an expected SII signature shall preserve their existing
identity/mailbox/DC/AL traffic shape and shall not gain implicit register reads.

### R4. Complete-bank clearing and mapping qualification

`MappingConfigController` shall provide an enhanced start path bound to both the
verified SyncManager bank and verified FMMU bank. Before writing any desired
SyncManager or FMMU entry it shall write a zero page to every discovered
SyncManager slot, read each slot back exactly, then perform the existing FMMU
bank clear/readback and desired mapping sequence.

The enhanced path shall reject a position/station mismatch, an
unverified/over-capacity bank, a configured SyncManager count greater than the
discovered count, or any configured SyncManager index outside the discovered
bank before emitting an action. Clear or configured readback mismatch shall
latch the mapping fault. Existing `start` and the FMMU-only enhanced path shall
remain source-compatible.

Startup shall provide a typed bridge that starts mapping for a position only
from the matching published SyncManager and FMMU banks, so production callers do
not reconstruct or trust independent register counts.

### R5. Documentation and capability boundaries

README, software/master requirements, capability manifest, and backend product
configuration spec shall describe the implemented software evidence and
complete-bank clearing. They shall continue to state that observed register
pages are evidence/reset bounds rather than desired product configuration and
that physical response authenticity, target timing/WCET, interoperability,
long-duration operation, and hardware HIL remain unqualified.

## Acceptance Criteria

- [x] A discovery unit test proves exact addresses, count/order, field decoding,
      control-pool ownership, zero-count completion, and complete-only
      publication.
- [x] Negative tests cover over-capacity count, wrong WKC, short payload, stale
      generation/action, timeout, and restart/reset behavior without partial
      evidence.
- [x] Startup tests prove expected-SII profiles emit FMMU reads, then
      SyncManager reads, then SII category-stream work before AL; all applicable
      evidence is published atomically and cleared on restart or terminal fault.
- [x] Mapping tests prove the enhanced path clears and reads back all discovered
      SyncManager slots, including unused slots, before FMMU reset and desired
      mapping; bank/count/index mismatch and clear readback mismatch fail closed.
- [x] Existing legacy `start` and FMMU-only mapping paths retain their traffic
      order and behavior.
- [x] A public Linux integration path proves the full verified-bank mapping
      controller remains compatible with the production scheduler/control pool.
- [x] `cargo test -p esop-ethercat-core --no-fail-fast` passes.
- [x] `cargo test -p esop-ethercat-linux-port --test scheduled_domains` passes.
- [x] `make ci` passes.
- [x] The final commit is pushed to `ra1mj/ESOP` and the exact pushed SHA has a
      successful GitHub Actions `quality` run.

Local acceptance evidence was collected on 2026-09-27. The focused core and
Linux integration suites passed, followed by the repository-wide `make ci`
gate covering formatting, diff checks, workspace all-feature tests, Clippy with
warnings denied, release and `aarch64-unknown-none` builds, BPF C syntax, and
the generated configuration/qualification validators.

## Verification Evidence

Verified on 2026-09-27:

- `cargo test -p esop-ethercat-core --no-fail-fast`: 269 core unit tests and
  all public core integration tests passed.
- `cargo test -p esop-ethercat-linux-port --test scheduled_domains`: 20
  production-scheduler simulation tests passed, including the public full
  verified FMMU/SyncManager bank clear path.
- `make ci` passed formatting/diff checks, workspace all-feature tests, Clippy
  with warnings denied, release and `aarch64-unknown-none` builds, BPF C syntax,
  deterministic configuration generation, and every qualification validator.
- Implementation commit `bacaa05fe4535fd3e8ff692365ef7e2ef596e3a7`
  passed GitHub Actions run `36295552710`, including Rust quality/live Zenoh,
  the CO-RE BPF build, and all privileged eBPF runtime qualification jobs:
  `https://github.com/ra1mj/ESOP/actions/runs/36295552710`.

## Out of Scope

- Deriving desired SyncManager configuration from observed live register pages.
- Mailbox Status Bit automatic discovery or FMMU mapping of that bit.
- Authenticating that a response came from the intended physical ESC.
- EtherCAT conformance, target-platform timing/WCET, or physical HIL claims.
- Vendor-specific SyncManager extensions beyond the standard eight-byte page.
