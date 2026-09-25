# Deterministic Product Configuration Generator

## Goal

Implement the first production-shaped `esop_cfggen` increment required by
FR-006 and ROB-011. A host-side tool shall compile a strict product manifest
and selected EtherCAT ESI device descriptions into deterministic, static
firmware configuration artifacts. Activated firmware must not need XML or
dynamic product discovery to obtain the generated topology, Domain/PDO plan,
CiA 402 axis policy, ProcBuf shape, or build-report inputs.

## Background

The repository already contains allocation-free activation/runtime contracts:
`DomainRegistry`, `Cia402PdoMap`, `Cia402AxisCommandPolicy`, ProcBuf ABI v6,
and a fail-closed `robot_build_report.json` envelope. The current build report
uses host placeholder values and the CiA 402 path still receives hand-written
per-axis policies. There is no tool that binds ESI identity/PDO evidence to a
product configuration hash and emits the static artifacts consumed by those
contracts.

## Functional Requirements

1. Add a host-only `esop-cfggen` binary with a versioned
   `esop.product.v1` JSON input contract. Unknown fields, unsupported schema
   versions, malformed values, and missing required sections must fail.
2. Parse the ESI subset needed for selected EtherCAT devices, including vendor
   ID, product code, revision, device name/type, RxPDO/TxPDO assignment index,
   SyncManager number, and PDO entry index/subindex/bit length.
3. Select an ESI device by explicit product identity and reject identity,
   revision, PDO assignment, duplicate object, direction, or bit-length
   mismatches. Namespace-qualified ESI XML must be accepted.
4. Validate unique product names, slave positions, station addresses, axis
   indices, Domain IDs, logical address ranges, process-image ranges, and
   datagram indices before publishing any output directory.
5. Build the product Domain/PDO/datagram/schedule plan through the existing
   `esop-ethercat-core` registration and frame-plan APIs rather than a private
   approximation. RxPDO entries must become output registrations and TxPDO
   entries input registrations with stable Domain-relative offsets.
6. Validate every configured drive axis through the existing
   `Cia402PdoMap` contract for its selected CSP, CSV, or CST mode. A drive
   missing any common, target, or actual object required by that mode must be
   rejected.
7. Validate and freeze each axis's signed position/velocity/torque scales,
   raw position offset, SI position range, velocity/torque limits, and
   per-cycle position step. Non-finite, zero-scale, inverted, negative, or raw
   overflow-prone policies must be rejected.
8. Generate atomically into a caller-selected directory:
   - `esop_product_config.h` with bounded static slave, Domain, PDO,
     datagram, and axis-policy arrays;
   - `product_config.json` with normalized machine-readable registrations and
     the complete source/config identity;
   - `device_inventory.json` with ESI identity and source SHA-256 evidence;
   - `procbuf_layout.json` with ABI version, dimensions, exact region bytes,
     and layout hash;
   - `robot_build_input.json` with topology, process-data, wire, WKC, copy,
     cycle, and ProcBuf resource values.
9. The configuration hash shall be SHA-256 over canonical normalized product
   semantics plus sorted ESI content hashes. It must not depend on source path,
   JSON key order, XML formatting, output directory, current time, or host.
10. Generation failure must leave an existing output directory unchanged.
    Successful replacement must not leave stale files from an older schema.
11. Extend `generate-robot-build-report.py` with an optional generated product
    input. When supplied, it shall use the generated config hash and product
    metrics while preserving fail-closed qualification semantics; an artifact
    alone must not claim HIL, resource, or cycle qualification.
12. Add a checked-in simulator example containing two CiA 402 drives and one
    EtherCAT IO device. Its generated artifacts shall be reproducible in CI,
    but explicitly unqualified for physical hardware.

## Safety and Determinism Requirements

1. The generator is host-only and may allocate, but generated cyclic data must
   remain fixed-size and directly representable by the existing `no_std`
   contracts.
2. No generated value may silently default an identity, scale, limit,
   Controlword/Statusword object, Domain address, WKC, or deadline.
3. The generator shall fail before output publication if the resulting
   process image, frame count, datagram count, ProcBuf dimensions, axis mask,
   or cycle budget exceeds the declared product envelope.
4. Expected WKC and wire-byte totals must be derived from the validated
   generated datagrams and frame plans, not copied from an unchecked report
   field.
5. Generated C identifiers and string literals must be escaped and bounded;
   arbitrary product text must not create compilable directives or duplicate
   symbols.

## Compatibility and Scope Requirements

1. Existing builds without `--product-input` must retain the current
   unqualified host build-report behavior.
2. ProcBuf ABI remains version 6; this task adds a runtime layout descriptor
   API that must exactly match the existing const-generic sizes and hashes.
3. The initial ESI subset may reject unsupported modular devices, alternative
   objects, bit-packed SyncManager layouts, vendor-specific scaling, and
   complex FMMU rules. Rejection must be explicit; the tool must not guess.
4. Generated simulator evidence is development evidence only and must not
   upgrade real-slave, HIL, WCET, functional-safety, or release qualification.

## Acceptance Criteria

- [x] `esop-cfggen` accepts the checked-in dual-axis simulator product and
      emits all five required artifacts.
- [x] Re-running generation from semantically identical JSON with reordered
      keys and reformatted ESI XML produces byte-identical normalized outputs
      and the same configuration hash.
- [x] ESI identity mismatch, duplicate slave/axis identity, missing selected
      PDO, wrong PDO direction/width, invalid axis policy, overlapping Domain
      address/image ranges, and resource overflow each fail with a specific
      nonzero error and publish no partial output.
- [x] Generated registrations pass the existing `DomainRegistry`, schedule,
      frame-plan, and selected-mode `Cia402PdoMap` validation paths.
- [x] ProcBuf runtime layout calculation matches `size_of` and
      `layout_hash::<...>()` for representative zero/small/product-limit
      dimensions.
- [x] The generated C header compiles with `gcc -std=c11 -Wall -Wextra
      -Werror` and exposes no zero-length product arrays.
- [x] `generate-robot-build-report.py --product-input ...` copies the exact
      generated hash/metrics and remains unqualified until independent cycle,
      target resource, and HIL evidence exists.
- [x] Unit/integration tests cover deterministic output, atomic replacement,
      strict schema rejection, ESI namespace parsing, and every fail-closed
      validation class above.
- [x] README, software PRD, robotics plan, configuration documentation,
      capability manifest, and Trellis backend specifications describe the
      implemented subset and its qualification boundaries.
- [x] `make ci`, cfggen tests, build-report tests, BPF build, and Zenoh tests
      pass locally.
- [x] Changes are committed, pushed to `ra1mj/ESOP`, and the resulting GitHub
      Actions run succeeds.

## Out of Scope

- Full ETG ESI/ENI language support, modular device profiles, FoE/SoE/EoE,
  SDO Information discovery, and vendor-specific object dictionaries.
- Automatic firmware compilation, flashing, physical EtherCAT activation, or
  proof that a real drive accepts the generated mapping.
- Certified WCET, physical wire timing, servo/mechanical suitability, FSoE,
  or functional-safety qualification.
- Replacing the existing runtime topology/SII read-back checks; generated
  identity and mapping remain expectations that runtime code must verify.
