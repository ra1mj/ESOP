# Design

## Boundaries

The change crosses four existing ownership boundaries without adding a new
runtime subsystem:

1. `esop-ethercat-core::sii` parses the category into a value-level model.
2. `SiiConfigurationCandidate` owns the observed fixed-capacity profile and
   includes it in the versioned structure signature.
3. `esop-cfggen` parses the ESI declaration and emits it into
   `esop-product-config`'s frozen slave record.
4. Product startup profile construction validates the profile against selected
   PDO groups, then builds the same signature Startup observes online.

## Data Model

- Add `MAX_SII_FMMU_USAGES = 16`, matching the bounded ESC-facing product model.
- Add `SiiFmmuUsage` with explicit wire codes for unused, outputs, inputs,
  SyncManager status, and unspecified (`0xff`).
- Add `SiiFmmuUsageProfile` containing `[SiiFmmuUsage; 16]` plus a count. It is
  `Copy`, allocation-free, exposes only the initialized slice, and appends
  atomically.
- Add `SiiCategory::fmmu_usages()` to validate category kind, non-empty payload,
  and every byte.

## Signature Contract

Advance `SII_CONFIGURATION_SIGNATURE_SCHEMA` from 1 to 2 and change the domain
separator to `esop.sii-configuration.v2`. The builder hashes an FMMU section
before PDO sections:

```text
0x20, count, usage[0..count]
```

The public signature carries `fmmu_count` for diagnostics. Existing callers may
build an empty FMMU section, but generated product profiles include their ESI
sequence. This is an intentional schema break: old generated artifacts must be
regenerated and cannot be mistaken for new evidence.

## ESI Contract

Only direct `Device/Fmmu` children are accepted. Text is normalized
case-insensitively and maps as follows:

| ESI text | SII code |
| --- | ---: |
| `Unused` | 0 |
| `Outputs` | 1 |
| `Inputs` | 2 |
| `MBoxState` / `SMStatus` | 3 |
| `Unspecified` | 255 |

The parser stores the ordered vector in `EsiDevice`; generator validation caps
it at 16 and emits a fixed profile into Rust and C. Semantic identity includes
the vector automatically through the serialized generated model.

## Mapping Compatibility

The product layer walks selected PDO groups in the same order used by
`SiiConfigurationCandidate::allocate_fmmus`: all Rx groups, then all Tx groups.
Each group consumes one FMMU index. The corresponding ESI usage must be
`Outputs` for Rx or `Inputs` for Tx. Extra declared FMMUs are retained in the
signature and may be unused, status, or unspecified; they are not programmed by
this task.

## Startup Gate

After stream finalization and before publishing either SII or DC evidence:

1. Read observed signature/profile from the candidate.
2. Reject when observed FMMU descriptor count exceeds the retained
   position-keyed ESC FMMU count copied from each completed `ScanRecord`.
3. Compare the full schema-v2 signature with the generated expectation.
4. Resolve and compare the selected DC descriptor.
5. Publish both evidence slots only after every enabled check succeeds.

This preserves the current atomic evidence rule and AL ordering.

## Compatibility And Rollback

- Generated example outputs are regenerated in the same commit.
- Profiles with no `<Fmmu>` declarations retain an empty FMMU section for
  explicit backward compatibility, but do not claim FMMU verification.
- Reverting the task is self-contained: remove the new profile fields and return
  signature schema/domain to v1. No persisted runtime state or migration exists.
