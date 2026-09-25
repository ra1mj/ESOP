# Design: Deterministic Product Configuration Generator

## Architecture

Add a host-only Rust binary crate, `crates/esop-cfggen`, while preserving all
existing runtime crates as `no_std`. The binary owns external document parsing,
normalization, deterministic hashing, artifact rendering, and atomic directory
publication. Validation delegates to existing runtime contract types whenever
possible.

```text
product.json + ESI XML
        |
        v
strict serde decode / quick-xml ESI subset parser
        |
        v
normalized product model + ESI content hashes
        |
        +--> axis policy validation
        +--> DomainRegistry / FramePlanSet / ScheduleTable
        +--> Cia402PdoMap::validate_for
        +--> ProcBuf runtime layout descriptor
        |
        v
staging directory -> fsync/rename-style atomic replacement
        |
        +--> esop_product_config.h
        +--> product_config.json
        +--> device_inventory.json
        +--> procbuf_layout.json
        +--> robot_build_input.json
```

## Input Contract

`esop.product.v1` contains:

- product identity: name, numeric robot ID, policy version;
- ProcBuf capacities: axes, IO channels, Domains, power-of-two events;
- base cycle period and deadline;
- Domain IDs, logical bases, period/phase ticks, and declared capacity limits;
- slaves: stable name, kind, position, station address, ESI path, expected
  vendor/product/revision/optional serial, Domain, and selected RxPDO/TxPDO
  assignment indices;
- axes: stable index/name, owning drive, selected cyclic mode, and complete
  `Cia402AxisCommandPolicy` values;
- platform/build metadata used by the build-report input without implying
  qualification.

All structures use `#[serde(deny_unknown_fields)]`. EtherCAT IDs are encoded as
canonical `0x` strings so their width and intent are explicit. Paths are
resolved relative to the product manifest, but normalized semantics and hashes
contain source labels plus file content hashes, never absolute host paths.

## ESI Subset

The parser streams XML with `quick-xml` and ignores namespace prefixes while
matching local element names. It extracts the root vendor ID and each Device's
Type product/revision identity, name, RxPDO/TxPDO assignment index, optional SM
number, and ordered Entry objects. It rejects:

- missing or ambiguous device identity;
- missing PDO assignment index or entry width;
- duplicate object/subindex in the same direction;
- non-byte-addressable selected PDO totals in this first version;
- unsupported numeric encodings or widths outside 1..=64.

The parser does not infer vendor units or policy limits from names/descriptions.

## Layout and Plan Construction

Within each Domain, selected RxPDO entries for all slaves are placed first,
followed by selected TxPDO entries. Each slave/direction starts on a byte
boundary. This provides one contiguous logical output segment and one
contiguous logical input segment per Domain while retaining stable PDO order.

The generator registers all entries with `DomainRegistry::register_pdo_at`,
then adds one `LWR` datagram for the output segment and one `LRD` datagram for
the input segment when present. Expected WKC is the count of participating
slave direction segments. Datagram indices are assigned globally in Domain
order. `activate_with_frame_plans` validates MTU/frame capacities and freezes
the schedule.

For each drive, the registered entries belonging to that slave are passed to
`Cia402PdoMap::from_pdo_entries` and `validate_for(selected_mode)`. Generic IO
devices retain their selected PDO registrations but do not receive a CiA 402
map.

## Axis Policy Validation

`esop-cfggen` adds a public host-usable validation constructor beside
`Cia402AxisCommandPolicy` rather than duplicating the lifecycle adapter's
rules. It checks finite nonzero signed scales, finite offset/ranges, ordered
position bounds, positive velocity/torque caps, positive per-cycle step, and
that each declared SI bound converts into the corresponding raw integer type.
The generated header stores the exact validated values.

## ProcBuf Runtime Descriptor

`esop-procbuf` gains an allocation-free runtime descriptor function over
`ProcBufDimensions`. It computes `CommandPage`, `StatePage`, event ring, and
complete region layouts using `core::alloc::Layout` and the actual fixed ABI
component layouts. It returns exact byte sizes plus the same hash inputs used
by the const-generic `layout_hash` function.

Tests compare the descriptor against `size_of::<ProcBuf<...>>()` and
`layout_hash::<...>()` across representative dimensions. The existing ABI v6
and const-generic API remain unchanged.

## Deterministic Identity

The configuration hash is SHA-256 of:

1. canonical JSON for the normalized semantic model with lexicographically
   sorted object keys and stable array order;
2. a sorted list of logical ESI labels and SHA-256 content hashes.

Generated timestamps, absolute paths, git commit, compiler, and output paths
are excluded. All JSON is rendered with stable indentation/newline rules, and
the C header follows normalized product order.

## Atomic Publication

Artifacts are written into a sibling temporary directory. Every artifact is
rendered and reopened/validated before publication. The old output directory
is renamed to a backup, the staging directory is renamed into place, and the
backup is removed only after success. On a publication error the previous
directory is restored. This also removes stale artifacts from older schemas.

## Build Report Integration

`generate-robot-build-report.py` accepts optional `--product-input` pointing to
`robot_build_input.json`. It validates the generated schema/config hash and
copies platform, device, process-data, cycle, and ProcBuf metrics. It still
sets `qualification.passed=false` unless independent qualification inputs are
introduced later. The existing no-input host behavior remains byte-shape
compatible.

## Rollback

The new crate, example files, and optional build-report argument are additive.
If rollout fails, remove the workspace member and optional integration; no ABI
or runtime cyclic behavior depends on generated artifacts yet. The ProcBuf
descriptor can remain as an independently tested reporting API.
