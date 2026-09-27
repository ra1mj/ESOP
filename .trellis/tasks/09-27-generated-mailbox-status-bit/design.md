# Design: generated mailbox status-bit discovery

## Existing Baseline

- `MailboxController` already supports a fixed-address one-byte Status Bit read
  before input-mailbox reads.
- `EsiSyncManager` already identifies MBoxOut and MBoxIn entries, but
  `EsiMailbox` discards their indices.
- Generated Rust currently emits `MailboxConfig::new(...)` only.
- Startup already reads the standard mailbox header and the complete SII
  category stream, and already verifies ordered FMMU usages.

## Data Model

Extend `EsiMailbox` with the selected send and receive SyncManager indices.
The receive control byte is already retained and will be used as additional
online SII evidence.

Add one core constructor for the canonical direct mailbox-full policy:

```text
status_address = ESC_SYNC_MANAGER_BASE
               + receive_sync_manager * ESC_SYNC_MANAGER_STRIDE
               + SYNC_MANAGER_STATUS_OFFSET
mask           = 0x08
active_high    = true
```

The constructor remains a `const`, infallible address derivation so generated
static data and existing manual callers stay source-compatible. The generated
receive-SyncManager descriptor validates its index against the runtime maximum
before the derived policy is accepted. All generators and product-validation
paths use the shared derivation instead of duplicating constants.

Generated product configuration retains the receive SyncManager index. The
presence of an ordered SyncManager-status FMMU usage determines whether the
canonical Status Bit is attached to `MailboxConfig`; otherwise `status_bit`
remains `None`.

## Validation Flow

1. ESI parsing selects exactly one MBoxOut and one MBoxIn descriptor and stores
   both indices.
2. cfggen determines whether the ESI FMMU usage list declares SyncManager
   status and emits the canonical status policy when it does.
3. `ProductConfiguration::validate` derives the expected policy again from the
   receive SyncManager index and ordered FMMU usages, then compares it with the
   embedded `MailboxConfig`.
4. `startup_profiles` passes the expected receive SyncManager descriptor to
   Startup together with the expected mailbox layout and SII signature.
5. Startup validates the standard mailbox header as today, then validates the
   indexed SII SyncManager entry against expected address, size, control, and
   enabled state before publishing the configuration.
6. Runtime PDO/mailbox batching receives the already validated
   status-enabled `MailboxConfig`; the existing controller performs the actual
   polling.

## Failure Policy

- Missing mailbox SyncManager indices, duplicate mailbox descriptors, invalid
  indices, or a non-canonical generated policy are configuration errors.
- Missing or mismatched online SII descriptors are Startup verification errors
  and prevent AL transition.
- No silent fallback from a declared Status Bit policy to PollTime is allowed.
- Absence of the capability declaration is a supported PollTime configuration,
  not an error.

## Compatibility

- Keep `MailboxConfig::new` and `with_status_bit` unchanged.
- New generated fields are additive in Rust and C artifacts.
- Existing ESI products without SyncManager-status usage retain their current
  runtime behavior.
- Runtime crates remain no_std and allocation-free.

## Test Strategy

- Core: canonical address derivation, bounds, and existing polling behavior.
- ESI/cfggen: indices retained, status policy emitted, PollTime fallback,
  deterministic output, malformed declarations rejected.
- Product config: canonical-policy and index tampering rejected.
- Startup: matching online SII accepted; missing, disabled, address/size/control
  mismatch rejected before transition.
- Production scheduler: inactive status suppresses mailbox read; active status
  permits it without caller-side configuration.
- Repository: focused tests, generated-artifact checks, no_std, clippy, and
  complete `make ci`.

## Deferred Design

An FMMU-mapped mailbox status bit requires a separate mapping contract: logical
address allocation, bit offset ownership, cyclic image extraction, and online
FMMU verification. This increment deliberately uses direct fixed-address SM
status-register reads and does not claim the mapped path.
