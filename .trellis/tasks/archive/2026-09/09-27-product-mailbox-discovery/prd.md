# Automate product mailbox configuration discovery

## Goal

Remove the remaining caller-supplied `MailboxConfig` requirement from generated
product PDO configuration. A validated ESI mailbox pair must become deterministic
product data, while the core also exposes a strict fixed-capacity parser for the
standard SII mailbox header so later online discovery can use the same direction and
capacity semantics.

This task provides software configuration evidence only. It does not claim that a
physical ESC returned the generated values, that live SII acquisition is integrated
into Startup, or that real-device mailbox/PDO interoperability is qualified.

## Background

- `esop-cfggen` currently parses ordered ESI `Sm` elements only for direction,
  `Enable`, and `OpOnly`; it discards mailbox start addresses and sizes.
- Generated product slaves therefore contain transition timeout and `OpOnly` data but
  no mailbox configuration.
- `StaticProductConfig::build_pdo_configuration_batch` requires a complete caller-built
  slice of position-keyed `ProductMailboxBinding` values even though the ESI has the
  owning device information.
- `MailboxController` already enforces a 128-byte fixed control payload and a six-byte
  mailbox header, so generated capacities must be checked against those runtime limits.
- SII standard mailbox words describe slave-receive/master-send and
  slave-send/master-receive ranges. Their direction must be converted explicitly rather
  than inferred from field names.

## Requirements

### ESI mailbox model

- Parse `StartAddress`, `DefaultSize`, and `ControlByte` from ordered device-level ESI
  `Sm` elements without changing PDO-local `Sm` handling.
- Recognize exactly one `MBoxOut` and one `MBoxIn` SyncManager for a mailbox-capable
  generated product slave.
- Treat `MBoxOut` as the master send/slave receive mailbox and `MBoxIn` as the master
  receive/slave send mailbox.
- Detect CoE support from a device-level `Mailbox/CoE` declaration because generated
  PDO assignment/mapping uses CoE SDO transactions.
- Reject missing or partial address/size attributes, duplicate mailbox directions,
  zero or undersized capacities, capacities above `MAX_MAILBOX_BYTES`, overflowing or
  overlapping ranges, disabled mailbox SyncManagers, and missing CoE support.
- Preserve parsed mailbox SyncManager control bytes in ESI semantics for future full SM
  descriptor generation, even though `MailboxConfig` currently needs only address and
  capacity.

### SII mailbox model

- Add named SII word constants for standard receive offset/size, standard send
  offset/size, and mailbox protocols.
- Parse exactly the standard mailbox fixed-header range from caller-owned words or a
  completed `SiiBlockReader` transaction.
- Expose protocol-bit helpers including CoE support and convert a valid CoE descriptor
  into the same `MailboxConfig` direction semantics used by ESI generation.
- Reject wrong block start/count, missing CoE, invalid capacities, address overflow,
  and overlapping ranges without partial output or allocation.

### Generated product propagation

- Carry the exact mailbox address/capacity pair through generated inventory,
  normalized product JSON, generated C/Rust configuration, ESI semantic hashes, and
  the final configuration hash.
- Add the generated `MailboxConfig` to each `ProductSlaveConfig`.
- Provide a no-argument fixed-capacity batch builder that uses only the generated
  per-slave mailbox data and returns no partial batch on validation failure.
- Preserve the existing position-keyed explicit binding API as an override path for
  tests, maintenance, or products that intentionally replace generated values.
- Validate generated and explicit configurations through one shared path so their
  capacity, range, station-address, job, and operation rules cannot drift.
- Preserve `no_std`, fixed arrays, caller-owned capacities, and allocation-free runtime
  behavior.

### Evidence and documentation

- Update the checked-in dual-drive plus IO ESI fixture with complete mailbox metadata
  and CoE declarations, regenerate all six deterministic artifacts, and update the
  compiled generated-product tests.
- Add focused ESI and SII tests for valid direction conversion and every fail-closed
  class above.
- Prove semantic/config hashes change when mailbox address or capacity changes and do
  not change for XML formatting-only edits.
- Update PRD, product configuration, EtherCAT requirements, capability manifest, and
  Trellis specs to distinguish generated mailbox completion from live SII/ESC/HIL
  validation that remains open.

## Acceptance Criteria

- [x] Complete ESI `MBoxOut`/`MBoxIn` metadata plus `Mailbox/CoE` produces one exact
      master send/receive `MailboxConfig` per generated slave.
- [x] Missing, duplicate, disabled, partial, too small, too large, overflowing,
      overlapping, or non-CoE mailbox declarations fail before artifact publication.
- [x] SII fixed-header parsing uses named word offsets, validates the exact block, maps
      slave receive/send fields to master send/receive fields, and exposes CoE support.
- [x] Invalid SII block shape, protocol, capacity, or address range returns a typed error
      and no configuration.
- [x] Generated JSON, inventory, C, Rust, ESI semantic hash, and configuration hash all
      include mailbox metadata deterministically.
- [x] `ProductSlaveConfig` owns generated mailbox data and can build the complete
      drive/drive/IO PDO batch without external bindings.
- [x] The explicit `ProductMailboxBinding` override path remains available and shares
      validation with the generated-data path.
- [x] Checked-in generated artifacts, focused tests, workspace formatting/lint/tests,
      `aarch64-unknown-none`, BPF, simulated HIL, and Zenoh integration gates pass.
- [x] Documentation states that live SII acquisition/cross-check, physical response
      authenticity, real-device interoperability, target WCET, and HIL remain open.

## Out Of Scope

- Adding live SII mailbox-word acquisition to `StartupController` or changing its scan
  phase sequence.
- Automatic full SyncManager/FMMU/DC descriptor generation or physical register
  readback.
- Mailbox Status Bit discovery, bootstrap mailbox use, FoE/SoE/EoE, or Complete Access.
- Physical EtherCAT hardware execution, ETG conformance, functional-safety claims, or
  release qualification.
