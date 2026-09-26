# Design: Startup-owned SII stream verification

## Architecture

The implementation extends the existing Startup FSM rather than adding a
parallel production service. Startup already owns identity, mailbox, and AL
ordering, so it is the only boundary that can prove no AL request was emitted
before live SII verification.

`StartupController<MAX_SLAVES>` keeps its public generic shape. It gains one
fixed-capacity `SiiStreamDiscoveryController` and caller-independent scratch
buffer reused for the current slave. This avoids propagating new const generics
through `ScheduledProductionServices` while retaining bounded memory and work.

The fixed Startup capacities are public named constants. The standalone
generic stream/discovery APIs remain available for products needing another
bound; exceeding the Startup bound is a typed pre-AL fault, not truncation.

## Signature Contract

`sii_config.rs` owns:

- `SiiConfigurationSignature`: Copy/Eq metadata plus a SHA-256 digest;
- `SiiConfigurationSignatureBuilder`: a transactional, allocation-free
  canonical encoder;
- candidate signature construction from validated mapping/layout evidence.

The signature starts with a schema/version domain separator. Integer fields
use explicit little-endian encodings and records use distinct tags. Canonical
order is structural SM summary, all RxPDO categories in observed/generated
order, then all TxPDO categories.

Each PDO record carries its index and SyncManager. Each entry record carries
object index, subindex, and bit length. The final signature also stores counts
outside the digest for diagnostics and defense in depth.

`SiiProcessDataSegment` gains `pdo_index`, `entry_start`, and `entry_count`.
Projection stages these fields together with layouts, so a category failure
cannot publish partial segment evidence.

## Generated Data Flow

1. ESI parsing already assigns contiguous SyncManager indexes and activation
   flags.
2. cfggen emits `sii_sync_manager_count` and `sii_enabled_sync_managers` per
   `ProductSlaveConfig` in normalized JSON, C, and Rust outputs.
3. `StaticProductConfig::startup_profiles()` validates the masks against the
   count, validates OpOnly as today, then builds a signature by grouping the
   product's flat PDO records by direction, assignment index, and SM.
4. The signature is copied into `StartupSlaveProfile.expected_sii`.

No generated digest is trusted blindly: runtime product attachment rebuilds it
from the same static fields it will use for PDO configuration.

## Startup State Flow

```text
ReadingIdentity
  -> ReadingMailbox (when configured)
  -> ReadingConfiguration (when expected_sii is present)
  -> TransitioningAl
```

`ReadingConfiguration` lazily starts a standard stream request for the current
station. `StartupAction::SiiConfiguration` wraps every EEPROM action. When END
is accepted, Startup finalizes the candidate into its fixed scratch buffer,
computes the live signature, and compares it with the profile expectation.

On match, `verified_sii[position]` is published and AL begins. On any error or
mismatch, Startup latches `Faulted`; no AL action can be returned.

The configuration stream uses `StartupConfig.sii_configuration_timeout_ns` as
one absolute per-slave scan deadline and `request_timeout_ns` for each action.

## Error Model

New typed errors distinguish:

- invalid generated signature/profile metadata;
- stream/discovery/projection errors;
- exact expected-versus-observed signature mismatch.

`SiiStreamDiscoveryController` already retains its first terminal error.
Startup maps it once and its fault phase returns no later action. Existing AL
first-fault diagnostics remain independent.

## Compatibility

- `StartupController<MAX_SLAVES>` and production scheduler type parameters are
  unchanged.
- `StartupSlaveProfile::EMPTY/new` default to `expected_sii: None`.
- Legacy `start()` and manually built profiles remain opt-out compatible.
- `ProductSlaveConfig` is a source-level additive field change; cfggen and all
  checked-in constructors/artifacts are updated atomically.

## Security And Safety Boundary

SHA-256 is used only as a fixed-size exact canonical comparison mechanism at
startup, not for authentication. Artifact trust, signed deployment, physical
response provenance, HIL, and functional-safety qualification remain outside
this task.

Physical process-data SM start/length/control fields are validated for internal
consistency by the live candidate but are not compared with ESI when ESI omits
them. Existing mailbox verification continues to compare the physical mailbox
ranges separately.

## Rollback

All new behavior is profile-gated. A rollback can omit `expected_sii` and retain
the prior path without changing scheduler ownership. No persistent state or ABI
migration is introduced.
