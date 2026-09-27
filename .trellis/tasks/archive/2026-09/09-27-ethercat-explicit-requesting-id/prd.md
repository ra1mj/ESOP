# EtherCAT Explicit Requesting ID

## Goal

Implement the product-configured EtherCAT Requesting ID mechanism so ESOP can
distinguish otherwise identical slaves before any mailbox, mapping, DC, or AL
activation work. This advances PRD FR-005 and master requirement CFG-010 while
remaining allocation-free and fail-closed.

## Background

ESOP already verifies position, fixed station address, vendor ID, product code,
revision, optional serial number, complete SII SM/PDO structure, and selected DC
descriptors. That is insufficient when two same-model devices have interchangeable
SII identities or serial matching is intentionally disabled.

The supported mechanism is EtherCAT Requesting ID through AL registers:

1. In INIT, write the current AL state plus ID Request bit 5 to AL Control
   `0x0120`.
2. Poll AL Status `0x0130` until ID Loaded bit 5 is set.
3. Read the 16-bit identification value from AL Status Code `0x0134`.

An ESI device declares this capability through
`Device/Info/IdentificationReg134`. Product configuration supplies the expected
per-position value. Both facts are independent and must agree before generation
or activation succeeds.

## Requirements

1. Add a fixed-capacity `no_std` Requesting ID state machine with typed actions,
   phases, progress, faults, absolute operation deadline, bounded per-request
   deadline, exact WKC 1, exact address/operation/generation/action ownership,
   and no sleeps or internal polling loops.
2. The state machine must preserve the first fault, publish a value only after
   write, loaded-bit observation, and exact two-byte read all succeed, and clear
   prior evidence on explicit restart.
3. Extend Startup with an optional per-slave expected Requesting ID. Validation
   must run after the existing SII identity check and before mailbox, live
   FMMU/SyncManager discovery, SII configuration, or AL transition actions.
4. A mismatch, malformed response, wrong WKC, wrong generation/action,
   unsupported AL state, timeout, or request-pool ownership error must leave
   Startup `Faulted`, prevent `Ready`, and expose a typed reason including the
   affected position where applicable.
5. Parse `Device/Info/IdentificationReg134` as a strict ESI boolean capability.
   Duplicate, misplaced, empty, or malformed declarations must be rejected.
6. Add an optional product manifest field for the expected 16-bit Requesting ID.
   Enabling it without ESI support must fail generation. ESI support alone must
   not silently enable the runtime check.
7. Propagate support and expected value through normalized product JSON, device
   inventory, generated C/Rust configuration, build input, semantic ESI hash,
   and final product configuration hash.
8. `esop-product-config` must revalidate enabled-implies-supported and rebuild
   the Startup profile from static product data, rejecting tampered support or
   expectation fields before Startup mutation.
9. Preserve compatibility for products that omit the new field: no Requesting
   ID wire action is generated and the existing identity-to-next-phase sequence
   remains unchanged.
10. Update capability and product/master/PRD documentation without claiming
    Station Alias identification, arbitrary Data Word identification, dynamic
    Hot Connect group discovery, physical response authenticity, HIL,
    conformance, or functional-safety qualification.

## Acceptance Criteria

- [x] Core tests cover the exact `0x0120 -> 0x0130 -> 0x0134` sequence, request
      bit/state payload, loaded-bit polling, successful value publication,
      loaded-bit delay, wrong WKC/shape/generation/action, timeout, mismatch, and
      restart evidence clearing.
- [x] Startup tests prove the Requesting ID gate occurs after SII identity and
      before mailbox/SII/AL work, and that two same-model positions with swapped
      or changed values cannot reach AL activation.
- [x] Generated-product tests cover supported+enabled, supported+disabled, and
      unsupported+enabled combinations plus runtime anti-tamper checks.
- [x] The checked-in simulator product uses distinct per-drive Requesting IDs,
      regenerated expected artifacts are deterministic, and the config hash
      changes only because the semantic configuration changed.
- [x] Linux production-scheduler integration proves Startup actions use the
      existing bounded control request pool and that mismatch blocks the
      lifecycle Topology gate before AL.
- [x] Relevant focused tests, `make test-hil`, and `make ci` pass.
- [x] Capability manifest and requirements documents bind the claim to software
      evidence and retain all hardware/qualification limitations.

## Out Of Scope

- Configured Station Alias as an identification mechanism.
- Arbitrary `IdentificationAdo` Data Word identification.
- Dynamic Hot Connect group discovery or topology insertion/removal.
- Physical-device response provenance, two-vendor interoperability, target
  WCET, long-duration HIL, ETG conformance, or functional safety.

## Delivery

- Verification passed on 2026-09-27 with the focused core, cfggen,
  product-config, Linux production-scheduler, build-report, R2 qualification,
  `make test-hil`, and full `make ci` gates.
- The pushed implementation/archive/journal chain through
  `77d0c8afee918128799eb914ae526618cbc22e93` passed GitHub Actions quality run
  `36325647352`.
