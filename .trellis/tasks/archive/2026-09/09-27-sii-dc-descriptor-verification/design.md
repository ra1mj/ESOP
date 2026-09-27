# Design: SII DC descriptor verification

## Data Flow

```text
ESI Device/Dc/OpMode[]
  -> strict EsiDcMode records
  -> product dc.op_mode selection
  -> generated SiiDcModeExpectation
  -> ProductSlaveConfig / StartupSlaveProfile

online SII category stream (single existing pass)
  -> borrowed Strings category + borrowed DC category
  -> exact name lookup
  -> observed SiiDcMode
  -> compare with expectation
  -> Startup verified DC evidence
```

The generated expectation and online observation meet only inside Startup.
cfggen owns XML and product-intent validation; the core owns SII byte decoding;
product-config owns static cross-field invariants; Startup owns live evidence
and pre-AL publication.

## Core SII Types

Add to `sii.rs`:

- `SII_DC_ENTRY_LEN = 24`.
- `SiiStringsCategory<'a>` with one-based exact byte lookup.
- `SiiDcCategory<'a>` with indexed fixed-entry decoding.
- `SiiDcMode`, a fixed numeric descriptor whose shift/factor fields are
  signed at protocol width.
- typed `SiiCategoryError` variants for invalid string/DC shape, index,
  reserved data, missing category, duplicate names and mismatch context.

The parser never stores borrowed data beyond the call. A helper scans a
complete SII image twice: first to locate the Strings category, then to locate
and validate the DC category. It compares the selected mode name as bytes and
returns a copied `SiiDcMode`.

`SiiDcModeExpectation` contains `&'static str` plus the copied numeric
descriptor. This keeps generated static data zero-allocation and lets Startup
compare the exact product-selected name without adding a dynamic string arena.

## ESI Parser

Extend `EsiDevice` with ordered `dc_modes: Vec<EsiDcMode>`. A dedicated
`DcModeBuilder` is active only under `Device/Dc/OpMode`, so `Name` and `Desc`
cannot collide with device, PDO, or entry fields.

Numeric mapping:

| ESI field | Generated/SII field |
| --- | --- |
| `CycleTimeSync0` text | `cycle_time0_ns: u32` |
| `CycleTimeSync0@Factor` | `sync0_cycle_factor: i16` |
| `ShiftTimeSync0` text | `shift_time0_ns: i32` |
| `CycleTimeSync1@Factor` | `sync1_cycle_factor: i16` |
| `ShiftTimeSync1` text | `shift_time1_ns: i32` |
| `AssignActivate` | `assign_activate: u16` |

Missing optional cycle/shift fields map to zero. Missing factors map to zero,
matching the SII field default. A non-zero `CycleTimeSync1` text value is
rejected because the selected runtime contract cannot verify it from this SII
category layout.

Duplicate names are rejected per device. Semantic serialization of
`EsiDevice` automatically includes all parsed modes and therefore updates the
existing ESI semantic hash without a parallel hash implementation.

## Product Selection And Generated Artifacts

`SlaveDcManifest` gains `op_mode: Option<String>`. During slave resolution:

- `required == true` requires a non-empty selector and an exact ESI name;
- `required == false` rejects a selector;
- reference-without-required remains invalid;
- the selected `EsiDcMode` is copied into `ResolvedSlave`/`GeneratedSlave` as
  an optional descriptor.

The descriptor is emitted in normalized JSON and inventory, added to the C
slave record, and emitted as `Option<SiiDcModeExpectation>` in generated Rust.
Because the canonical semantic object already contains generated slaves, the
selection and descriptor participate in the configuration hash.

## Runtime And Startup

`ProductSlaveConfig` adds `sii_dc_mode: Option<SiiDcModeExpectation>`.
`startup_profiles()` validates the relationship with `dc_required` and calls
`with_expected_dc_mode` on the profile.

`StartupSlaveProfile` and `StartupController` add expected/verified DC slots.
The configuration stream is required when either the existing SII signature
or DC expectation is present. `finish_sii_configuration` keeps the existing
candidate/signature verification, then validates the DC mode from the same
scratch image. Both checks must succeed before either piece of per-position
evidence is committed.

To preserve transactional publication, expected checks are staged in locals.
Only after all checks succeed are `verified_sii[current_index]` and
`verified_dc_modes[current_index]` updated and the controller advanced.
Restart resets both arrays.

## Compatibility And Failure Semantics

- Products without required DC remain source-compatible at the manifest
  level because `op_mode` is optional.
- Rust construction of `ProductSlaveConfig` is intentionally additive and all
  checked-in callers/generated artifacts are updated together.
- Existing SII stream action ownership, generation, deadline, WKC and scratch
  capacity behavior is unchanged.
- Descriptor failures are reported as typed Startup/SII errors and preserve
  the first terminal fault.
- No SII parser result is treated as proof that the physical response came
  from the intended device.

## Verification

Unit tests isolate byte parsing and ESI parsing. cfggen golden/regeneration
tests prove deterministic artifact propagation. Product-config tests prove
static invariant handling. Startup tests drive the real SII action sequence
and assert that no AL action appears before DC verification succeeds.

The repository gates remain `cargo fmt`, focused workspace tests, no-std
target checks, `make ci`, `make bpf`, and exact-SHA GitHub Actions.
