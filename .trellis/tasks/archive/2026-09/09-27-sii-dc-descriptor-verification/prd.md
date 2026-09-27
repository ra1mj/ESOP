# SII DC descriptor verification

## Goal

Close the next discovery boundary of PRD FR-018 / CFG-002 / CFG-007 by
making the generated product configuration select one explicit ESI
Distributed Clocks operation mode for every DC-required slave and making
Startup verify the matching SII category `0x003c` descriptor before the first
AL transition.

The increment shall preserve the existing bounded SII stream and
SyncManager/PDO verification path, add no activated-cycle work, and publish
position-keyed DC descriptor evidence only after the complete online SII image
has been decoded and matched.

## Background

- Online scan already proves ESC System Time capability and Startup publishes
  the selected reference clock plus propagation-delay topology.
- `DcClockController` already initializes System Time Offset/Delay, while the
  existing single-station `DcController` can program SYNC0/SYNC1 when given a
  complete `DcConfig`.
- Product manifests currently carry only `required` and `reference_clock`.
  They do not identify which ESI `Dc/OpMode` owns the intended
  `AssignActivate`, cycle factors, or shifts.
- Startup already reads the full SII category stream once and compares a
  generated SyncManager/PDO signature before AL. The same completed image is
  the correct source for a DC descriptor comparison; a second EEPROM pass is
  unnecessary.
- The standard SII DC category uses fixed 24-byte entries. The software model
  needs only the numeric fields and selected mode name required for an exact
  cross-source comparison; it must not infer a mode from slave kind.

## Requirements

### R1. Strict SII DC and string decoding

1. `SiiCategory` shall expose typed parsers for the Strings category
   `0x000a` and DC category `0x003c` without allocation.
2. A DC payload shall contain one or more exact 24-byte entries. Each entry
   shall decode little-endian SYNC0 cycle time, signed SYNC0 shift, signed
   SYNC1 shift, signed SYNC1 cycle factor, `AssignActivate`, signed SYNC0
   cycle factor, one-based name/description string indices, and four reserved
   bytes.
3. Invalid category kind, zero/non-multiple length, non-zero reserved bytes,
   truncated string entries, invalid one-based string indices, and duplicate
   selected mode names shall return typed errors without partial publication.
4. String resolution shall remain borrowed from the caller-owned SII image;
   no heap allocation, text normalization, or UTF-8 conversion is permitted
   in the core parser. Comparison uses exact ESI UTF-8 bytes.

### R2. ESI DC operation-mode parsing and selection

1. The strict cfggen ESI subset shall parse every `Device/Dc/OpMode` in source
   order, including required `Name` and `AssignActivate`, optional `Desc`,
   optional SYNC0 cycle/shift, optional SYNC1 cycle/shift, and signed `Factor`
   attributes.
2. Missing required fields, duplicate mode names, invalid integer widths,
   unsupported non-zero direct `CycleTimeSync1` values, or nested/malformed DC
   elements shall fail before output publication.
3. `esop.product.v1` shall add optional `dc.op_mode`. A DC-required slave must
   name exactly one ESI mode; a non-required slave may omit it and shall not
   publish an expected DC descriptor. Reference-clock policy continues to
   imply `required`.
4. Generated inventory, normalized JSON, C, Rust, ESI semantic hash, and final
   configuration hash shall carry the selected descriptor deterministically.
   XML/JSON formatting and input path changes shall not alter semantic output.

### R3. Fixed runtime expectation contract

1. `esop-ethercat-core` shall define a fixed-size, `Copy`, `no_std`
   expectation/evidence type containing the selected mode name and all SII-
   representable numeric descriptor fields.
2. `ProductSlaveConfig` shall carry `Option<SiiDcModeExpectation>` and
   `StaticProductConfig::startup_profiles()` shall reject DC-required slaves
   without an expectation and non-required slaves with one.
3. Product startup profile construction shall remain transactional: invalid
   DC selection returns before Startup is mutated.
4. Existing products that do not require DC and omit `dc.op_mode` shall retain
   their previous behavior.

### R4. Startup online verification and evidence

1. A profile with an expected DC mode shall use the existing
   `ReadingConfiguration` pass. Startup shall not issue a second SII scan.
2. After the stream and existing SyncManager/PDO projection complete, Startup
   shall locate the Strings and DC categories, resolve the selected mode by
   exact name, and compare every SII-representable numeric field.
3. Missing Strings/DC categories, unknown or duplicate mode names, malformed
   descriptors, or any field mismatch shall fault before any AL action and
   shall publish no DC descriptor evidence for that position.
4. Successful verification shall publish immutable position-keyed observed
   descriptor evidence. Restart shall clear it together with the existing
   mailbox/SII evidence.
5. Existing profiles without an expected DC descriptor shall preserve the
   current SII-signature-only path.

### R5. Scope and claim boundary

1. Core changes remain `no_std`, fixed-capacity, non-blocking, allocation-free
   and outside the activated PDO cycle.
2. Generic control/RX code shall remain unaware of SII category semantics.
3. Documentation and capability claims shall distinguish descriptor
   discovery/verification from SYNC start-time calculation, topology-wide
   register programming, application-time authenticity, runtime clock lock,
   physical timing precision, HIL, conformance, and functional safety.

## Acceptance Criteria

- [x] Core unit tests cover multi-entry Strings/DC categories, signed fields,
      exact mode lookup, malformed lengths, reserved bytes, missing strings,
      unknown/duplicate names, and field mismatch.
- [x] cfggen tests cover ESI multi-mode parsing, explicit product selection,
      missing/unknown/duplicate modes, factor width, unsupported direct
      SYNC1 cycle values, deterministic hashes, and generated artifact fields.
- [x] Product-config tests prove expectation invariants and exact propagation
      into `StartupSlaveProfile` before any Startup mutation.
- [x] Startup tests prove one-pass SII verification, successful position-keyed
      evidence, pre-AL mismatch failure, restart clearing, and compatibility
      for profiles without DC expectations.
- [x] The checked-in dual-axis simulator selects one named DC mode for both
      drives while the IO slave remains without a DC descriptor expectation.
- [x] Generated artifacts, README, requirements, product documentation,
      backend spec, and capability manifest describe the implemented boundary
      without claiming all-slave SYNC programming or physical qualification.
- [x] Focused tests, no-std checks, repository `make ci`, `make bpf`, and
      GitHub Actions pass on the exact pushed commit.

## Out Of Scope

- Calculating a common first SYNC trigger/start time.
- Programming `0x0980`, `0x0990`, `0x09a0`, or `0x09a4` for every slave.
- Choosing the cycle period from runtime scheduler state or scaling ESI
  factors against the product base period.
- External PTP/TAI/application-time discipline, periodic all-slave drift
  compensation, sync-window monitoring, clock-lock qualification, or recovery.
- Physical response provenance, device interoperability, target WCET,
  long-duration HIL, ETG conformance, or functional-safety qualification.

## Technical Notes

- The parser is an independent implementation based on the public category
  field contract. External source study is behavior evidence only; no source
  code, structure layout, comments, or test vectors are copied into ESOP.
- Direct `CycleTimeSync1` content is not represented in the 24-byte SII entry.
  The selected strict subset accepts only zero content and records the signed
  factor that is actually available online.
- This task intentionally supplies the verified input contract for the next
  independently testable increment: generated topology-wide SYNC0/SYNC1 plan
  construction and common start-time programming.
