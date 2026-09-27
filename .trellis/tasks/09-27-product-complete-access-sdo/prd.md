# Product-controlled Complete Access SDO

## Goal

Complete the software path for FR-017 / MBX-002 by adding a typed, fail-closed
CANopen-over-EtherCAT Complete Access contract. Complete Access shall be usable
only when the selected ESI device declares the capability and the product
manifest explicitly enables it for that slave. The generated policy must
survive every artifact boundary and be revalidated before runtime use.

## Background

- `SdoTransfer` already accepts an untyped boolean named `complete_access`, but
  currently encodes it as `0x80`, which modifies the command specifier instead
  of setting the CoE Complete Access bit `0x10`.
- Product generation currently records only generic CoE mailbox support and
  cannot distinguish ESI capability from product authorization.
- PDO assignment/mapping uses single-subindex SDO transfers and must remain on
  that path in this increment.
- SDO Information is the remaining half of FR-017 and is deliberately deferred
  to the next focused increment.

## Requirements

1. `esop-ethercat-core` shall own a typed SDO access mode and a policy that
   defaults to single-subindex access. Callers shall not pass raw booleans.
2. A Complete Access request shall fail before transfer state changes when the
   bound policy disables it or when the requested subindex is greater than 1.
3. Initiate-upload and initiate-download requests shall encode Complete Access
   with command bit `0x10`. Segmented follow-up requests shall retain their
   existing wire format.
4. Complete Access upload responses shall be checked for an access-mode match.
   Standard `0x60` initiate-download responses shall remain accepted because
   the response does not mirror the request's Complete Access bit.
5. ESI parsing shall capture the direct `Device/Mailbox/CoE` boolean
   `CompleteAccess` capability, defaulting to false when absent and rejecting
   malformed boolean values.
6. The strict `esop.product.v1` per-slave contract shall accept an optional
   `coe` object with `complete_access`, defaulting to disabled. Enabling it for
   an ESI device that does not advertise support shall fail generation.
7. ESI support and product enablement shall be emitted in normalized JSON,
   inventory, generated C/Rust, runtime input, semantic identity, and config
   identity. ESI support without product enablement is valid and remains
   disabled at runtime.
8. `esop-product-config` shall rebuild one typed SDO access policy per slave,
   reject generated `enabled => !supported` tampering, and expose the validated
   policy for constructing an `SdoTransfer`.
9. Existing PDO configuration shall explicitly request single-subindex access.
   No automatic Complete Access grouping shall be introduced.
10. Changes shall remain deterministic, fixed-capacity, allocation-free in
    target crates, and compatible with `no_std`.

## Acceptance Criteria

- [x] Complete Access upload, expedited download, and segmented download
      initiate requests use bit `0x10` and exact expected command bytes.
- [x] Disabled policy, unsupported product enablement, invalid Complete Access
      subindex, and upload response-mode mismatch fail with typed errors.
- [x] Complete Access downloads accept the standard `0x60` response and
      segmented transfers complete without adding the bit to segment commands.
- [x] The simulator demonstrates ESI capability enabled for both drives,
      product authorization enabled for one drive, and disabled for the other.
- [x] Generated JSON, inventory, C, Rust, runtime input, semantic hash, and
      config hash represent support and enablement deterministically.
- [x] Runtime activation reconstructs the generated policy and rejects
      enabled-on-unsupported or otherwise inconsistent generated data.
- [x] A mailbox/master integration test observes the exact Complete Access
      request on the wire path.
- [x] Existing PDO configuration, ordinary SDO, mailbox, startup, and generated
      artifact tests remain green.
- [x] Focused tests, generated artifact checks, `no_std`, clippy, and full
      `make ci` pass.
- [x] PRD, requirements, capability evidence, and Trellis product configuration
      contract are updated without claiming HIL or ETG conformance.

## Out Of Scope

- SDO Information service discovery or object-dictionary enumeration.
- Automatic Complete Access grouping for PDO assignment or PDO mapping.
- SDO block transfer support.
- Real-slave interoperability, target WCET, long-duration testing, ETG
  qualification, or functional-safety certification.

## References

- `docs/esop-software-prd.md`: FR-017
- `docs/ethercat-master-requirements.md`: MBX-002
- `crates/esop-ethercat-core/src/coe.rs`
- `.trellis/spec/backend/product-configuration.md`
