# SII FMMU usage descriptor verification

## Goal

Close the software-only portion of the PRD's remaining FMMU discovery gap by
verifying each slave's standard SII FMMU usage descriptor against the selected
ESI/product configuration before Startup emits its first AL transition action.
The master must continue to own logical-address allocation; this task verifies
the slave's declared FMMU index capabilities and does not claim address
automatic discovery or hardware qualification.

## Background

- The SII category stream is already read from word `0x0040` through the END
  marker with bounded storage, then projected atomically into
  `SiiConfigurationCandidate`.
- Startup currently compares a versioned SM/PDO structure signature and the
  selected DC mode before any AL action, but ignores category `0x0028`.
- The mapping layer already assigns one FMMU per non-empty PDO segment in
  deterministic RxPDO-then-TxPDO order and writes/read-backs the resulting ESC
  register images.
- ESI parsing and generated product configuration currently retain SM/PDO/DC
  information but do not retain repeated `<Fmmu>` declarations.
- SOEM's public SII parser treats category `0x0028` as one function byte per
  FMMU index: `0` unused, `1` outputs, `2` inputs, `3` SyncManager status;
  `0xff` is an unspecified/default marker.

## Requirements

### FMMU-001 Core SII model

The core shall expose a fixed-capacity, allocation-free FMMU usage model for
the standard SII category. Parsing shall preserve index order and accept only
the standard values `0`, `1`, `2`, `3`, and `0xff`. Unknown values, duplicate
categories, empty categories, and capacity overflow shall fail without
publishing a partial candidate.

### FMMU-002 Versioned configuration evidence

`SiiConfigurationSignature` shall advance to a new schema and cover the exact
ordered FMMU usage sequence in addition to the existing SM/PDO structure. The
builder shall remain usable for profiles that intentionally omit FMMU evidence,
while profiles that declare FMMUs must mismatch a missing, reordered, or
different online category.

### FMMU-003 ESI and generated product propagation

The ESI parser shall accept repeated direct `Device/Fmmu` elements with the
standard names `Unused`, `Outputs`, `Inputs`, `MBoxState`, and an explicit
unspecified marker. It shall reject unknown names and excess entries. The
semantic JSON, generated Rust product configuration, generated C header/source,
and committed example artifacts shall retain the ordered fixed-capacity profile.

### FMMU-004 Startup fail-closed gate

Startup shall verify that the observed SII FMMU descriptor length does not
exceed the FMMU count reported by the ESC scan record. It shall compare the new
signature before any AL action and publish neither SII nor DC evidence when the
FMMU count or signature check fails. Error reporting shall identify the slave
position and observed/reported count where applicable.

### FMMU-005 Product mapping consistency

Generated/product startup profile construction shall reject a selected PDO
layout whose deterministic FMMU index sequence is incompatible with the ESI
FMMU usage profile. RxPDO groups require `Outputs`; TxPDO groups require
`Inputs`; unused, SyncManager-status, unspecified, or missing entries cannot
satisfy a process-data FMMU index.

### FMMU-006 Documentation and traceability

The PRD, EtherCAT requirements, implementation status documents, capability
manifest, and project README shall describe the delivered capability precisely:
SII FMMU usage/index verification is implemented, while logical-address
allocation remains master-owned and physical response/HIL qualification remains
open.

## Acceptance Criteria

- [ ] Unit tests parse every supported usage, preserve order, and reject empty,
      duplicate, unknown, and over-capacity categories atomically.
- [ ] Signature tests prove FMMU count, order, usage, missing-category, and
      schema changes affect equality/digest as designed.
- [ ] ESI/generator tests prove `<Fmmu>` values reach semantic JSON, generated
      Rust, and generated C artifacts; invalid values and excess entries fail.
- [ ] Product tests reject Rx/Tx PDO groups mapped onto incompatible FMMU usage
      indexes and accept the committed dual-axis example.
- [ ] Startup simulation proves a valid descriptor reaches the existing AL
      sequence and count/signature failures emit no AL action and no evidence.
- [ ] Existing SM/PDO/DC, mapping, product generation, Linux simulation, and
      workspace tests remain green.
- [ ] `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`,
      `cargo test --workspace`, repository policy checks, and exact pushed-SHA
      GitHub Actions complete successfully.

## Out Of Scope

- Discovering or changing master logical addresses from SII.
- Automatic FMMU register programming beyond the existing mapping controller.
- Mailbox Status Bit scheduling changes.
- Physical provenance, ETG conformance, real-slave interoperability, and HIL.
- Increasing the existing process-data mapping model's segment capacity.
