# FMMU-mapped mailbox status bit

## Goal

Complete the software path for FR-016 / MBX-004 that maps each declared
`MBoxState` input-mailbox-full bit into the generated cyclic process image.
After the slave mapping is active, production mailbox polling shall consume
validated Domain input instead of issuing a separate SyncManager status-byte
read, while PREOP/PDO configuration retains the existing direct-register
bootstrap path.

## Background

- ESI/SII ordered FMMU usage, canonical direct MBoxIn status-byte discovery,
  Startup evidence, complete FMMU/SyncManager register-bank discovery, Mapping
  write/readback, generated Domain plans, and fixed-capacity scheduled Domain
  input are already implemented.
- A product declaring `MBoxState` currently receives only the direct policy
  `0x0800 + sm * 8 + 5`, mask `0x08`, active-high.
- PDO/CoE configuration runs before Mapping, so the mapped bit cannot replace
  direct polling during bootstrap.

## Requirements

1. For every product slave with one supported SyncManager-status FMMU usage,
   cfggen shall allocate one deterministic input bit after that Domain's PDO
   input region. Bindings are ordered by slave position and packed into the
   minimum byte-aligned tail of the Domain image.
2. The generated binding shall identify the slave position, Domain, Domain bit
   offset, maximum observation age, ordered FMMU index, logical bit, canonical
   MBoxIn status-register physical bit, read direction, and enabled state.
3. The Domain input datagram shall cover the packed status tail and its
   expected WKC shall include every mapped status FMMU. Generated JSON,
   inventory, C, Rust, cycle metrics, semantic hash, and config hash shall all
   reflect the mapping.
4. Product validation shall reconstruct the deterministic placement and FMMU
   descriptor from PDO layout, Domain timing, slave order, MBoxIn index, and
   ordered FMMU usages. Missing, duplicate, overlapping, out-of-range, stale,
   or tampered mappings shall fail closed before activation.
5. The validated binding shall expose the exact `FmmuConfig` needed by the
   existing Mapping controller. Mapping write/readback remains the authority
   that activates the mapped bit; observed startup register pages are evidence
   and reset bounds, not product configuration.
6. `MailboxController` shall support an explicit mapped-status polling mode.
   Its existing `start` behavior remains PollTime/direct-register compatible;
   mapped mode shall never emit a direct status read.
7. The production scheduler shall read a mapped observation only from the
   configured Domain and only when its committed input quality is valid and
   its age is below the generated Domain-period bound. Inactive, unavailable,
   invalid, or stale observations shall suppress the input-mailbox read and
   shall not silently fall back to direct polling.
8. PDO configuration mailboxes shall continue to use the direct status byte
   before Mapping completes. Runtime callers switch to mapped mode explicitly
   after the product Mapping controller reaches `Complete`.
9. Products without SyncManager-status usage shall retain PollTime behavior.
   Existing manually configured direct Status Bit users remain source and
   behavior compatible.
10. Runtime changes shall remain deterministic, fixed-capacity,
    allocation-free, and `no_std` compatible.

## Acceptance Criteria

- [x] The simulator product generates two packed mailbox status bits at the
      deterministic tail of the motion Domain input image.
- [x] Generated Domain length, LRD payload, expected WKC, inventory, C/Rust
      artifacts, semantic hash, and config hash include the mapped status tail.
- [x] Product activation rejects altered Domain/bit/FMMU index/logical address,
      physical address/bit, direction, age, duplicate placement, missing input
      datagram coverage, or inconsistent `MBoxState` declaration.
- [x] The exact generated mailbox-status `FmmuConfig` can be added to a
      per-slave Mapping table and passes the existing write/readback FSM.
- [x] A mapped-mode mailbox transaction never emits a fixed-address status
      read; inactive status suppresses mailbox input reads and active status
      permits the next bounded mailbox read.
- [x] Invalid WKC/Domain quality and an observation older than the Domain
      period suppress mailbox input reads without direct-register fallback.
- [x] PDO configuration and existing direct Status Bit/PollTime tests remain
      green.
- [x] Focused tests, generated artifact checks, `no_std`, clippy, and full
      `make ci` pass.
- [x] Product/compliance documentation distinguishes software completion from
      physical response authenticity, interoperability, WCET, and HIL.

## Out Of Scope

- Automatic construction and scheduling of complete multi-slave PDO Mapping
  batches beyond exposing and testing the canonical status FMMU descriptor.
- Proving EtherCAT physical response provenance, real-slave interoperability,
  target WCET, long-duration behavior, ETG conformance, or functional safety.
- Replacing the direct bootstrap policy before Mapping is active.

## References

- `docs/esop-software-prd.md`: FR-016
- `docs/ethercat-master-requirements.md`: MBX-004
- `.trellis/tasks/archive/2026-09/09-27-generated-mailbox-status-bit/`
