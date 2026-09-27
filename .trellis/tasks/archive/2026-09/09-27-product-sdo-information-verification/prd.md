# Product-controlled SDO Information verification

## Goal

Complete the software portion of FR-017 / COE-004 by adding a bounded,
product-controlled CoE SDO Information path that verifies the object metadata
for every selected PDO entry against the generated ESI contract before the
result can be trusted by runtime code.

## Background

- `CoeService::SdoInformation` exists, but the core has no request/response
  codec, transfer state machine, or object-dictionary verifier for service
  `0x08`.
- The selected ESI PDO entries currently retain index, subindex, bit length,
  signedness, and name, but discard the exact CANopen data-type code needed for
  an SDO Information comparison.
- Product configuration already carries a fail-closed Complete Access policy.
  SDO Information is a separate ESI capability and product authorization and
  must not be inferred from generic CoE or Complete Access support.
- The PRD explicitly excludes a full object-dictionary browser. The useful
  product boundary is the finite set of objects selected into generated PDOs.

## Requirements

1. `esop-ethercat-core` shall provide a `no_std`, allocation-free SDO
   Information codec and transfer state machine for OD-list, object-description,
   entry-description, and error responses.
2. Requests shall use exact CoE SDO Information opcodes and validate service,
   opcode, object identity, fragment ordering/countdown, response shape, and
   abort code before publishing a result.
3. Fragment processing shall be bounded. Object indices use caller-selected
   fixed capacity; object and entry descriptions retain only the fixed metadata
   needed by the verifier and safely consume bounded optional trailing data.
4. The core shall expose typed CANopen data types, object codes, value-info
   flags, access rights, and RxPDO/TxPDO mappability instead of raw protocol
   integers at verification call sites.
5. The core shall provide a fixed-capacity verifier for generated PDO-entry
   expectations. It shall request one object description per object, then one
   entry description per expected subindex, and publish success only after the
   complete plan matches.
6. Verification shall reject an absent object/subindex, data-type mismatch,
   bit-length mismatch, missing required SDO read/write access, missing PDO
   mappability, duplicate/invalid expectations, capacity excess, malformed or
   out-of-order fragments, and SDO Information aborts with typed errors.
7. ESI parsing shall capture the direct `Device/Mailbox/CoE@SdoInfo` boolean,
   default it to false when absent, and reject malformed boolean values. It
   shall preserve the exact supported CANopen data type for each PDO entry.
8. The strict `esop.product.v1` per-slave `coe` object shall accept
   `sdo_information`, defaulting to false. Cfggen shall reject enablement when
   the selected ESI device does not advertise support.
9. Cfggen shall derive one deterministic, deduplicated expectation set from
   the selected RxPDO and TxPDO entries. Rx entries require write access and
   RxPDO mappability; Tx entries require read access and TxPDO mappability.
10. ESI support, product enablement, and every expected entry shall survive
    normalized JSON, inventory, robot build input, generated C/Rust, semantic
    identity, and configuration identity without lossy re-derivation.
11. `esop-product-config` shall revalidate `enabled => supported`, expectation
    shape/capacity/uniqueness, and per-slave ownership before exposing a typed
    SDO Information policy and plan. Activation shall fail before publishing
    runtime state when generated data is inconsistent.
12. Products without `coe.sdo_information` shall retain current behavior and
    issue no SDO Information request. The path shall remain outside the cyclic
    PDO data plane and use the existing mailbox/control scheduling boundary.

## Acceptance Criteria

- [x] Exact-byte tests cover OD-list, object-description, and entry-description
      requests plus successful, fragmented, error, malformed, and out-of-order
      responses.
- [x] Typed parsers expose object data type/max subindex/object code and entry
      data type/bit length/access/PDO mapping without heap allocation.
- [x] A verifier succeeds for a multi-object generated expectation plan and
      fails closed for every mismatch listed in Requirement 6 without partial
      publication.
- [x] ESI tests cover absent, true, false, paired/empty CoE elements, malformed
      `SdoInfo`, supported data types, and unsupported data types.
- [x] Product schema tests cover default-disabled, supported-enabled,
      supported-disabled, unsupported-enabled, and unknown-field behavior.
- [x] The simulator advertises SDO Information and the product enables it for
      one drive only; the generated expectation set matches its selected PDOs.
- [x] Generated JSON, inventory, robot build input, C, Rust, semantic hash, and
      config hash represent support, enablement, and expectations
      deterministically.
- [x] Product runtime policy/plan lookup and activation anti-tamper tests reject
      enabled-on-unsupported, duplicate, invalid, or wrong-owner entries.
- [x] A mailbox/master integration test observes an exact SDO Information
      request and drives a complete object/entry verification sequence.
- [x] Existing ordinary/Complete Access SDO, PDO configuration, mailbox,
      startup, generated artifact, and product activation tests remain green.
- [x] Focused tests, generated checks, `make no-std`, `make lint`, and full
      `make ci` pass.
- [x] FR-017 / COE-004, capability evidence, product configuration docs, and
      Trellis contracts state the implemented software boundary without
      claiming real-slave, HIL, ETG, WCET, or safety qualification.

## Out Of Scope

- An interactive or unbounded full object-dictionary browser.
- Runtime ESI/XML parsing or dynamic heap-backed dictionary storage.
- SDO block transfer or changes to cyclic PDO scheduling.
- Automatically changing PDO mappings based on discovered metadata.
- Physical-device interoperability, target timing/WCET, long-duration HIL,
  EtherCAT conformance, or functional-safety certification.

## References

- `docs/esop-software-prd.md`: FR-017
- `docs/ethercat-master-requirements.md`: COE-004
- `docs/esop-etg-cia402-master-requirements.md`: mailbox/CoE P1 boundary
- `crates/esop-ethercat-core/src/coe.rs`
- `crates/esop-cfggen/src/esi.rs`
- `.trellis/spec/backend/product-configuration.md`
