# Design: ESC base and DC capability discovery

## Architecture

The change extends the existing `ScanController` and `StartupController`. It
does not add another production service or a parallel request owner.

```text
APRD base block 0x0000
  -> exact ScanRecord base fields
  -> APWR fixed station address
  -> FPRD 0x0910 when Features Supported says DC
  -> FPRD ESC Configuration / AL Status
  -> Startup validates product DC policy
  -> deterministic reference station evidence
  -> existing identity/SII/AL path
```

## Core Data Contract

`registers.rs` owns the base-block size and the Features Supported bit masks.
The scanner reads the same bounded base range used by iGH and decodes:

- type: byte `0`;
- revision: byte `1`;
- build: little-endian bytes `2..4`;
- FMMU count: byte `4`;
- SyncManager count: byte `5`;
- RAM size: byte `6`, widened only for API convenience;
- port descriptor: byte `7`;
- Features Supported: little-endian bytes `8..10`.

`EscDcRange` is an explicit enum (`Bits32`, `Bits64`). `ScanDcCapabilities`
contains the raw feature word, FMMU bit-operation support, base DC support,
range, System Time support, and the first sampled value. It provides a single
`can_be_reference_clock()` predicate so Startup and later DC code do not
duplicate the capability rule.

`ScanRecord` changes `esc_type` to `u8` and `build` to `u16`; no in-repository
consumer depends on the old incorrect shape.

## Scan State Flow

`ReadingDcSystemTime` is inserted after station assignment. It is entered only
when the base feature bit advertises DC. The request uses the fixed station
address and reads four or eight bytes according to the range bit.

This action has capability-probe WKC semantics:

- `1`: validate exact payload length, decode the sample, mark System Time;
- `0`: accept an empty/untrusted data result as “delay only” and continue;
- `>1`: terminal `UnexpectedWorkingCounter`.

All other scan phases retain exact WKC 1 semantics. The public action exposes
whether zero is an accepted capability-negative result so transport tests and
callers do not infer policy from the phase.

## Startup Contract

`StartupDcRequirement` is a compact enum stored in `StartupSlaveProfile`:

- `None`;
- `SystemTime`;
- `ReferenceClock`.

After scan completes and before starting identity reads, Startup validates all
profile positions against scan records in one bounded pass. It stages the
selected reference locally and publishes it only after the complete pass
succeeds. Multiple explicit references are rejected before any product
evidence is published.

If no profile nominates a reference, the first scan-order record for which
`can_be_reference_clock()` is true is selected. This is evidence for later
configuration, not proof that delay/offset setup is complete.

## Generated Data Flow

The strict product schema adds:

```json
"dc": {
  "required": true,
  "reference_clock": true
}
```

The object defaults to both false. Reference implies required and only one
slave may select reference. Validation happens before generated artifacts are
staged. The normalized model, C struct, generated Rust static module, semantic
hash, and configuration hash all carry the values.

`ProductSlaveConfig` keeps the two booleans because generated code is static
data. `startup_profiles()` converts them to the core enum after validating the
same invariant defensively.

## Compatibility And Migration

- Omitted product `dc` objects preserve prior behavior.
- `StartupSlaveProfile::new/EMPTY` default to `None`.
- Legacy `StartupController::start()` creates no DC requirements but still
  records live capability and picks the first eligible reference when present.
- Generated example artifacts change atomically with cfggen and runtime types.
- No ProcBuf or external protobuf ABI changes are introduced.

## Failure And Evidence Semantics

Scan failures retain the existing terminal first error. Startup capability
validation is transactional: no selected reference is visible if any required
profile fails. Restart resets all capability-derived selection.

WKC 0 is accepted only for the System Time capability probe and never means a
valid clock sample. It cannot satisfy a product requirement or be selected as
reference.

## Rollback

The behavior is profile-gated. Product manifests can omit `dc`, and Startup
profiles can use `None`. A code rollback must revert the base-field type
correction and generated artifacts together; retaining the old mixed-field
decoder is not an acceptable compatibility mode.
