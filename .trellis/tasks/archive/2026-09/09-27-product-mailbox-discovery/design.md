# Design: generated product mailbox configuration

## 1. Shared mailbox range validation

Add a public, allocation-free mailbox configuration validator in
`esop-ethercat-core`. It validates both directions against `MAILBOX_HEADER_LEN` and
`MAX_MAILBOX_BYTES`, checks non-zero physical addresses, checked end addresses, and
non-overlap. `MailboxController::start` reuses this validator before applying its
payload-specific limit.

All ESI, SII, generated-product, and explicit-override paths construct the same
`MailboxConfig` and use this validator. Timing, retry, and optional status-bit policy
remain the current deterministic defaults unless a caller intentionally applies an
override.

## 2. ESI parsing

Extend `EsiSyncManager` with optional `start_address`, `default_size`, and
`control_byte` fields parsed with checked integer conversion. Device-level `Mailbox`
state records whether a `CoE` child was present, including self-closing `CoE` elements.

At `DeviceBuilder::finish`, classify ordered SyncManagers:

- `MBoxOut`: slave receive, therefore `MailboxConfig.send_*`;
- `MBoxIn`: slave send, therefore `MailboxConfig.receive_*`;
- `Outputs`/`Inputs`: existing process-data behavior.

A generated product slave must have one enabled, complete pair and CoE support. The
pair is converted to `EsiMailboxConfig`, validated through the core mailbox contract,
and retained on `EsiDevice`. Non-mailbox process-data SyncManagers may omit physical
metadata until the separate full descriptor task.

## 3. SII fixed-header parser

Add constants for words `0x001C..0x0020` and an `SiiStandardMailbox` value containing
the two ranges and protocol mask. Its constructors accept either:

1. an exact five-word slice beginning at the standard receive offset word; or
2. a completed `SiiBlockReader` whose recorded start and count match that range.

The parser keeps SII naming internally, exposes protocol helpers, and provides
`coe_mailbox_config()` for explicit master-direction conversion plus shared validation.
It performs no I/O and does not mutate Startup.

## 4. Generator propagation

`GeneratedSlave` gains a serializable mailbox value. `EsiDevice` participates in the
semantic ESI hash, and the normalized product semantic identity includes the
label-to-ESI-hash map. Therefore any mailbox address/capacity/control/protocol change
affects the ESI semantic hash and the final product configuration hash.

The generated C slave struct gains four mailbox fields. Generated Rust imports
`MailboxConfig` and initializes `ProductSlaveConfig.mailbox_config`. JSON inventory and
normalized product output receive the same nested value without adding a seventh
artifact.

## 5. Product runtime API

`ProductSlaveConfig` gains `mailbox_config`. Add
`build_generated_pdo_configuration_batch()` to build jobs in frozen slave order.
Keep `build_pdo_configuration_batch(bindings)` as the explicit override API.

Both delegate to a private batch builder that resolves one configuration per slave and
validates it before pushing any job. Generated data uses the exact embedded value;
explicit data retains missing/duplicate/unknown binding errors. Add a typed invalid
mailbox error carrying slave position and the core validation error.

## 6. Compatibility and rollback

- Existing explicit-binding callers remain source-compatible except that direct
  `ProductSlaveConfig` literals must initialize the new required field.
- Generated artifacts intentionally change hash and bytes because mailbox semantics are
  now part of the product contract.
- Rollback is a single task commit: no persisted runtime state or migration format is
  introduced.
- Live SII comparison remains a later activation feature; documentation must not call
  this physical discovery or validation.
