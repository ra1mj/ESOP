# Product Configuration Contract

## 1. Scope / Trigger

Use this contract when changing `esop-cfggen`, `esop-product-config`,
`esop.product.v1`, ProcBuf runtime layout reporting, generated C/Rust/JSON
artifacts, runtime product attachment, or product build-report projection. The
generator is host-only; generated cyclic data must remain directly
representable by fixed-capacity `no_std` contracts.

## 2. Signatures

```text
esop-cfggen --input <product.json> --output <directory>
make cfggen-example
make cfggen-runtime-example
make build-report PRODUCT_INPUT=<directory>/robot_build_input.json
make cfggen-build-report
```

Rust entry points:

```rust
pub fn generate(input: &Path, output: &Path) -> Result<GenerationSummary>;
pub fn describe_layout(
    dimensions: ProcBufDimensions,
) -> Result<ProcBufLayoutDescriptor, ProcBufLayoutError>;
pub fn Cia402AxisCommandPolicy::validate_for_product(
    self,
) -> Result<(), Cia402AxisCommandPolicyError>;
pub fn StaticProductConfig::activate(...) ->
    Result<ActivatedProduct<...>, ProductActivationError>;
pub fn StaticProductConfig::sdo_access_policy(position: u16) ->
    Result<SdoAccessPolicy, ProductSdoAccessPolicyError>;
pub fn ActivatedProduct::slave_copy_plans(...) ->
    &SlaveCopyPlanSet<MAX_PRODUCT_SLAVE_COPIES>;
pub fn ScheduledDomainBank::publish_slave_copies(
    plans: &SlaveCopyPlanSet<PLANS>,
    image: &mut SlaveCopyProcessImage<BYTES>,
    target_cycle: u64,
) -> Result<SlaveCopyPublication<PLANS>, ScheduledSlaveCopyError>;
pub fn ScheduledProductionCycleOwner::with_slave_copies(
    ...,
    plans: &'a SlaveCopyPlanSet<PLANS>,
    image: &SlaveCopyProcessImage<BYTES>,
) -> Result<Self, ScheduledProductionOwnerError>;
pub fn SlaveCopyProcessImage::published_image(&self) -> SlaveCopyPublishedImage<'_>;
pub fn StaticProductConfig::build_pdo_startup_plan<const OPS: usize>(...) ->
    Result<ProductPdoStartupPlan<OPS>, ProductPdoPlanError>;
pub fn StaticProductConfig::build_pdo_configuration_batch<
    const JOBS: usize,
    const OPS: usize,
>(...) -> Result<PdoConfigBatchPlan<JOBS, OPS>, ProductPdoBatchError>;
pub fn StaticProductConfig::build_generated_pdo_configuration_batch<
    const JOBS: usize,
    const OPS: usize,
>() -> Result<PdoConfigBatchPlan<JOBS, OPS>, ProductPdoBatchError>;
pub fn StaticProductConfig::startup_profiles(...) ->
    Result<[StartupSlaveProfile; SLAVES], ProductStartupError>;
pub fn StaticProductConfig::watchdog_plan() ->
    Result<WatchdogPlan<SLAVES>, ProductStartupError>;
pub fn StaticProductConfig::start_startup(...) -> Result<(), ProductStartupError>;
pub enum StartupDcRequirement { None, SystemTime, ReferenceClock }
pub fn StartupController::selected_reference_clock(...) ->
    Option<StartupReferenceClock>;
pub fn StartupController::dc_capabilities(position: u16) ->
    Option<ScanDcCapabilities>;
pub fn StartupController::dc_topology(...) ->
    Option<&DcTopology<MAX_SLAVES>>;
pub fn StartupController::verified_fmmu_registers(position: u16) ->
    Option<FmmuRegisterBank>;
pub fn StartupController::verified_sync_manager_registers(position: u16) ->
    Option<SyncManagerRegisterBank>;
pub fn StartupController::start_mapping_for_position<const SMS: usize, const FMMUS: usize>(
    ...,
    table: &MappingTable<SMS, FMMUS>,
) -> Result<(), StartupError>;
pub fn FmmuRegisterDiscoveryController::start(...) ->
    Result<(), FmmuRegisterDiscoveryError>;
pub fn SyncManagerRegisterDiscoveryController::start(...) ->
    Result<(), SyncManagerRegisterDiscoveryError>;
pub fn MappingConfigController::<SMS, FMMUS>::start_with_verified_fmmus(
    ...,
    bank: FmmuRegisterBank,
    table: &MappingTable<SMS, FMMUS>,
) -> Result<(), MappingConfigError>;
pub fn MappingConfigController::<SMS, FMMUS>::start_with_verified_registers(
    ...,
    sync_manager_bank: SyncManagerRegisterBank,
    fmmu_bank: FmmuRegisterBank,
    table: &MappingTable<SMS, FMMUS>,
) -> Result<(), MappingConfigError>;
pub fn DcTopology::<MAX_SLAVES>::build(
    records: &[ScanRecord],
    reference_position: Option<u16>,
) -> Result<DcTopology<MAX_SLAVES>, DcTopologyError>;
pub fn DcClockController::<MAX_SLAVES>::start(
    config: DcClockConfig,
    topology: &DcTopology<MAX_SLAVES>,
    generation: u16,
    application_time_ns: u64,
    monotonic_now_ns: u64,
) -> Result<(), DcClockError>;
pub fn DcSyncTiming::resolve(
    base_period_ns: u64,
    mode: SiiDcMode,
) -> Result<DcSyncTiming, DcSyncTimingError>;
pub fn StaticProductConfig::dc_sync_plan() ->
    Result<DcSyncPlan<SLAVES>, ProductStartupError>;
pub fn DcSyncController::<MAX_SLAVES>::start(
    config: DcSyncConfig,
    plan: &DcSyncPlan<MAX_SLAVES>,
    topology: &DcTopology<MAX_SLAVES>,
    generation: u16,
    monotonic_now_ns: u64,
) -> Result<(), DcSyncError>;
pub fn ScheduledProductionServices::with_dc_clock_configuration(
    controller: &mut DcClockController<MAX_SLAVES>,
) -> ScheduledProductionServices<...>;
pub fn ScheduledProductionServices::with_dc_sync_configuration(
    controller: &mut DcSyncController<MAX_SLAVES>,
) -> ScheduledProductionServices<...>;
pub fn WatchdogController::<MAX_SLAVES>::start(
    config: WatchdogControllerConfig,
    plan: &WatchdogPlan<MAX_SLAVES>,
    generation: u16,
    monotonic_now_ns: u64,
) -> Result<(), WatchdogError>;
pub fn ScheduledProductionServices::with_watchdog_configuration(
    controller: &mut WatchdogController<MAX_SLAVES>,
) -> ScheduledProductionServices<...>;
```

## 3. Contracts

Input schema `esop.product.v1` is strict (`deny_unknown_fields`) and owns:

- product/robot/policy identity;
- ProcBuf dimensions and generator capacities;
- period/deadline and platform metadata;
- ordered Domain, slave, selected ESI PDO, and axis policy declarations;
- an optional ordered `slave_copies` list. Each strict entry names one source
  TxPDO, target RxPDO, and unsigned one-byte target quality RxPDO by slave name,
  object index, and subindex, plus the product-reviewed fallback byte;
- an optional strict per-slave `dc` object whose `required` and
  `reference_clock` booleans default false, where reference implies required,
  at most one slave may select reference, every required slave selects one
  exact ESI `Dc/OpMode` by `op_mode`, and a non-required slave selects none.
- an optional strict per-slave `watchdog` object whose `divider` and
  `process_data_intervals` raw `u16` values are independently optional, where
  the object contains at least one field and every present value is nonzero;
  absence preserves the corresponding ESC default and emits no action.
- an optional strict per-slave `coe` object whose `complete_access` boolean
  defaults false. It may be true only when the selected direct
  `Device/Mailbox/CoE` declares a valid `CompleteAccess=true` capability.

ESI paths are confined relative paths and their canonical targets must remain
under the product directory. Names/labels rendered into C are nonempty,
NUL-free, and at most 128 UTF-8 bytes.
Because `Name`, `Desc`, and timing element names overlap other ESI scopes, the
DC builder may consume them only as direct `Device/Dc/OpMode` children. Nested
`OpMode` records, wrapper elements around DC fields, or an `OpMode` outside
that exact path must fail instead of falling through to Device/PDO parsing.

Successful generation atomically replaces the output directory with exactly:

```text
esop_product_config.h
esop_product_config.rs
product_config.json
device_inventory.json
procbuf_layout.json
robot_build_input.json
```

`esop_product_config.rs` contains only static data and the public/re-exported
`esop-product-config` API. Runtime activation checks `esop.product-runtime.v1`,
the caller-expected 32-byte hash, exact ProcBuf v7 descriptor/header, exact
observed online/configured topology, rebuilt Domain/PDO/datagram/WKC evidence,
schedule/frame plans, drive ownership, product policies, and selected-mode
CiA 402 PDO maps.
For slave copies, cfggen first resolves semantic references within the selected
ESI PDOs and validates them with `SlaveCopyPlan::build` against the active
generator registry. Generated Rust stores the resolved PDO array indices;
runtime activation retains only the referenced registration handles, rebuilds
the fixed-capacity plan set from its own active registry, and rejects unknown
indices, direction/width/alignment/datagram-coverage errors, capacity excess,
or overlapping target/quality bytes. It returns the owning frozen result only
after all checks pass.

`copy_bytes_per_cycle` is the maximum sum of configured slave-copy payload
widths whose target Domains are due on one hyperperiod tick. It is not the
Domain input-byte count and excludes the separate target quality-byte write.

The activated plan set is executed only by the stable production-cycle owner.
After verified cycle N receive completion, the owner asks the active
`ScheduledDomainBank` to preflight every plan due for N+1, stage from the last
published process-image page, apply value/fallback and quality writes in plan
order, and atomically publish one new page. Any missing binding, image mismatch,
cycle replay, or plan error publishes nothing and moves the owner to a terminal
fault phase. Plans targeting the lifecycle-owned motion Domain are rejected at
owner construction.

An auxiliary frame backed by the shared process image requires the exact typed
`SlaveCopyPublishedImage` returned by that publisher. The lifecycle boundary
checks the target cycle, image length, and backing-page address before any TX
mutation. Static auxiliary images and empty-copy products retain their existing
paths; a detached raw slice cannot authorize a shared-image output.

Each generated slave carries four ESM transition timeout classes, one validated
CoE mailbox pair, one bounded `OpOnlySyncManagerProfile`, and an ordered FMMU
usage profile with an explicit count plus a fixed 16-entry array. Only direct
`Device/Fmmu` declarations are accepted; unknown, misplaced, empty, or excess
declarations fail before staging. ESI `MBoxOut`
maps to master-send/slave-receive and `MBoxIn` maps to
master-receive/slave-send. Both SyncManagers must be enabled and provide
`StartAddress`, `DefaultSize`, and `ControlByte`, while the Device must declare
`Mailbox/CoE`. Missing, duplicate, overlapping, overflowing, undersized, or
oversized mailbox ranges fail before publication. Timeout values are decimal
milliseconds converted to checked nanoseconds; missing values use the named
`ETG1020_DEFAULT_TRANSITION_TIMEOUTS_V1`. Mailbox data, timeout profiles, and
activation templates and the ordered FMMU usage profile participate in ESI
semantic and configuration hashes.
The direct CoE element may also declare the strict boolean `CompleteAccess`
capability. Missing means unsupported and malformed values fail parsing. ESI
support and product enablement remain separate generated booleans in normalized
JSON, inventory, C/Rust and build input. ESI support participates in semantic
and configuration hashes; product enablement participates in configuration
identity. Generation rejects enablement without support. Runtime activation
rechecks the same implication before ProcBuf/topology mutation, and
`sdo_access_policy(position)` is the only product helper that converts those
fields into a core `SdoAccessPolicy`. Policy-aware transfers use
`SdoTransfer::with_policy`; default transfers and all generated PDO
assignment/mapping operations remain explicitly `SdoAccess::Single`.
The generator also retains both mailbox SyncManager indexes. An ordered
SyncManager-status / `MBoxState` FMMU usage requires the canonical direct
MBoxIn status policy `0x0800 + index * 8 + 5`, mask `0x08`, active-high;
absence retains PollTime. JSON, inventory, C and Rust must expose the indexes
and derived policy, and no product input may override its address, mask or
polarity. For every declared status usage, cfggen also appends one bit to the
owning Domain after its byte-aligned PDO input region, ordered by slave
position. The tail is byte-aligned once, the aggregate LRD covers it, and
input WKC increases once per mapped status FMMU. The generated binding freezes
Domain bit offset, maximum age equal to the Domain period, ordered FMMU index,
logical bit, canonical physical bit 3, read type and enable state. Those fields
participate in every artifact and semantic/configuration hash.
All ordered ESI DC mode metadata participates in the ESI semantic hash. The
product-selected mode, its exact SII-representable descriptor, and the resolved
absolute SYNC0/SYNC1 cycle, signed SYNC0 shift, and exact 16-bit
`AssignActivate` participate in the normalized product, inventory, generated
C/Rust and configuration hash. One shared `DcSyncTiming::resolve` contract owns
the checked direct/factor arithmetic; cfggen and the `no_std` runtime boundary
must not implement private timing formulas.

The core SII parser accepts only the exact five-word standard mailbox header,
checks CoE support and the same address/capacity rules, and performs the same
slave-to-master direction conversion. It parses caller-owned words or a
completed exact-range `SiiBlockReader`. A Startup profile may carry the
generated expected mailbox; after identity verification and before any AL
transition, Startup reads exactly SII words `0x001C..0x0020`, requires CoE,
compares only the four physical address/capacity fields, and stages rather
than publishes the expected runtime policy. The later SII category comparison
must verify the generated MBoxIn SyncManager index, address, length, control
byte and enabled state, while ordered FMMU usage verifies whether the
canonical direct Status Bit is permitted. Only then may Startup publish the
complete position-keyed MailboxConfig. Runtime poll, timeout, retry and
status-bit policy are not SII-header layout fields. Profiles without an expected mailbox and the
legacy `start` API retain the identity-to-AL path.

The core also provides a fixed-capacity SII category stream contract.
`SiiCategoryStreamReader<WORDS>` starts at standard word `0x0040` by default,
reuses `SiiBlockReader` register transactions, and reads two-word headers plus
their declared payloads until `SII_CATEGORY_END`. Internal continuations keep
one absolute scan deadline and do not reset action-token or datagram-index
cursors. The image, category count, and byte projection stay unavailable until
END is accepted. Capacity, missing-END, address, WKC, generation, payload and
timeout failures are typed and latch the first terminal error. The companion
`SiiStreamDiscoveryController` publishes `SiiConfigurationCandidate` only
after caller-owned scratch conversion and complete transactional FMMU usage/
SM/RxPDO/TxPDO projection; explicit PDO signedness is preserved. The standard
FMMU category must be non-empty, unique, at most 16 bytes, and contain only
unused, outputs, inputs, SyncManager-status, or unspecified values. The
schema-v2 SHA-256 structural signature covers the exact ordered FMMU sequence,
SM count/enabled/OpOnly masks and ordered Rx-then-Tx PDO
index/SM/object/subindex/bit-length records; signedness is intentionally
excluded from live comparison. The same completed caller-owned
image exposes borrowed Strings and fixed 24-byte DC category parsers. Mode
names use exact one-based string indices, reserved bytes must be zero, and all
signed shifts/factors retain their protocol widths without allocation.

`startup_profiles` validates timeout values, exact slave positions, generated
mailbox ranges, the MBoxIn descriptor, canonical direct Status Bit policy,
generated FMMU count, generated SM count/enabled mask, OpOnly
flags, exclusive selected RxPDO ownership, and contiguous PDO groups before
rebuilding the expected SII signature from the static fields used by runtime
configuration. In deterministic all-Rx-then-all-Tx group order, every process
data FMMU index must declare Outputs for Rx or Inputs for Tx; missing or other
usage values fail before Startup mutation. A SyncManager-status declaration
requires the canonical Status Bit, while its absence requires PollTime; no
silent fallback is allowed. Generated PDO batches repeat the same validation
before exposing jobs to the production scheduler. Product activation also
reconstructs every mapped status tail and exact FMMU descriptor from PDO
layout, Domain timing, slave order, MBoxIn index and ordered usage. It rejects
missing, duplicate, out-of-bounds, stale-policy, Domain/datagram/WKC or
descriptor drift before publishing position-keyed bindings. The existing
Mapping controller remains the only writer/readback authority. PDO
configuration retains direct polling; mapped mailbox mode is selected
explicitly after Mapping Complete and consumes only fresh valid committed
Domain input, without direct-register fallback.
It first validates the product-wide DC invariant and maps each static policy to
`StartupDcRequirement::{None,SystemTime,ReferenceClock}`; invalid policy must
return before Startup mutation. It also propagates the selected
`SiiDcModeExpectation` only for required slaves. The DC policy and selected
descriptor are present in normalized JSON, device inventory, generated C/Rust
and the product configuration SHA-256.
`start_startup` supplies those profiles to Startup. After identity and optional
mailbox verification, profiles with an expected SII signature first read every
ESC-reported standard 16-byte FMMU page in index order, then every standard
8-byte SyncManager page, and only then enter the bounded configuration stream
phase. Both register controllers use one absolute deadline, exact WKC 1/length/
generation/action ownership and fixed maxima of 16 pages. Startup stages both
position/station-bound banks together with structural-signature and selected-DC
observations, then publishes all applicable evidence records only after every
comparison succeeds. It rejects online FMMU usage or SyncManager counts that
exceed the matching scanned count. It emits the first AL action only after exact
comparison; register, stream, projection, capacity, timeout, ownership,
signature, missing category, malformed descriptor, unknown mode, or field
mismatch faults publish no FMMU/SyncManager/SII/DC evidence. Profiles without
an expected SII signature retain their legacy traffic shape. A nonzero legacy
`StartupConfig.transition_timeout_ns` is an explicit uniform override;
otherwise each AL step selects its generated/default timeout. OpOnly and AL
work for one step share one absolute deadline. Non-OP and leaving-OP paths
disable/read back every OpOnly output before readiness or AL transition;
entering OP enables/read backs only after OP is observed. Any mismatch, WKC,
generation, length, or timeout fault blocks Ready.

Before identity, mailbox, category-stream, or AL work, Startup consumes the
completed position-keyed scan records. The scanner reads the exact 12-byte ESC
base range from `0x0000`, decodes type/revision/build/FMMU/SM/RAM/port/features
at protocol widths, and probes fixed-address System Time `0x0910` with four or
eight bytes according to Features Supported. WKC 1 publishes a sample; WKC 0
is capability-negative and means delay-only; every other WKC or response fault
latches scan. Every base-DC slave, including delay-only WKC-0 devices, then
reads exactly 16 bytes at `0x0900`; every slave reads exactly two Data Link
Status bytes at `0x0110`. Both reads require exact WKC 1 and exact payload
length before a `ScanRecord` is published. The record retains four little-
endian receive times plus raw and decoded link-up, loop-closed and signal flags.

Startup stages an explicit reference or first capable fallback locally, builds
one `DcTopology<MAX_SLAVES>` from complete records, and publishes both only
after all policy and delay checks pass. The physical tree consumes scan order
with port 0 upstream and downstream order `[3, 1, 2]`. Delay construction uses
32-bit wrapping timestamp subtraction, checked aggregate arithmetic and
symmetric measurable DC-to-DC edges; an iterative graph walk publishes checked
cumulative delay from the selected reference. Optional unmeasurable DC remains
`None`, while `SystemTime` or `ReferenceClock` requirements without cumulative
delay latch Startup before identity. Restart or any Startup fault clears both
selected reference and topology.

Clock initialization consumes only this published immutable topology. Before
emitting a request, `DcClockController` validates the selected reference and
every System-Time-capable slave's range and cumulative delay. It reads exactly
24 bytes from `0x0910`, advances the caller's paired application-time sample by
checked monotonic elapsed time, applies a 32-bit wrapping or checked 64-bit
signed correction to the old raw offset, then writes one coherent 12-byte
offset-plus-delay payload at `0x0920`. The selected reference delay is zero;
every other programmed delay comes from topology evidence. Exact action,
generation, response length, WKC 1 and deadline checks are mandatory. Public
programmed evidence remains empty until the whole plan completes, while a
fault may retain only a diagnostic completed count because accepted ESC writes
cannot be rolled back. This controller does not authenticate the application
time, prove runtime lock, or prove physical response origin or timing precision.

`StaticProductConfig::dc_sync_plan()` rebuilds every generated timing value
through the shared resolver before Startup mutation, rejects raw/resolved mode
drift, preserves product position/station order, omits non-DC slaves, and
requires the unique configured reference to belong to the plan and immutable
Startup topology. `DcSyncController` validates the complete plan before its
first action, disables `0x0981` on every planned slave, writes the two cycle
registers at `0x09a0`, reads reference `0x0910` once, computes a strictly-future
LCM-aligned common epoch with bounded request lead, writes each shifted start at
`0x0990`, and finally writes the exact `AssignActivate` word at `0x0980`.
Public per-slave evidence remains empty until every final activation succeeds;
restart clears all staged and public evidence without claiming accepted
hardware writes were rolled back.

`StaticProductConfig::watchdog_plan()` revalidates every generated watchdog
value before Startup mutation, preserves generated slave order and binds the
exact position/station address. `WatchdogController` programs each present
field as a two-byte little-endian fixed-address write followed by an exact
two-byte readback: divider at `0x0400/2`, then process-data intervals at
`0x0420/2`. Every action requires WKC 1, exact length, generation/action/pool
ownership, one absolute configuration deadline and bounded request deadlines.
Programmed evidence stays private until the whole plan completes; any fault
latches the first typed error and clears the published slice. An empty plan is
an immediate successful no-op.

Runtime sync-window monitoring is an activation-time attachment, not another
product artifact. Build `DcSyncWindowConfig` only after Startup has published
the immutable `DcTopology`; the topology helper derives the exact nonzero BRD
WKC from its slave count. Its datagram index and four-byte process-image region
must be disjoint from the reference FRMW and from every Domain/control owner
before `prepare` mutates state. The cyclic owner stages reference time and the
`0x092c/4` broadcast word under one generation and publishes neither monitor
until both validate. The lower 31 bits are an aggregate maximum difference,
not per-slave identity. A product may freeze threshold and hysteresis values in
its integration layer, but this increment does not add them to the generated
schema, change ProcBuf layout, or authorize automatic clock correction.

Per-slave PDO startup-plan construction uses the same generated order and the
shared 256-entry cfggen bound. For each SyncManager it clears assignment
subindex zero, writes each mapping object, then publishes the ordered mapping
indexes and final assignment count. `build_generated_pdo_configuration_batch`
builds one fixed-capacity batch from generated per-slave mailbox values in
frozen product order. The position-keyed `ProductMailboxBinding` API remains an
explicit override path. Both paths share mailbox range validation,
duplicate-station checks, job/operation capacities, and every per-slave plan
before returning any batch.

`PdoConfigBatch` owns the existing `PdoConfigController` and
`MailboxController`, derives one generation per job, and advances only after
exact download/upload readback. Empty jobs are skipped with a loop bounded by
the static job capacity; faults retain the exact job until explicit restart.
The caller still owns batch start timing, while the production scheduler owns
mailbox transport, retry policy, batch
advancement, and CONFIGURING lifecycle admission. The caller may opt
`StartupConfig` into a PREOP barrier for PDO Configuration, Watchdog
Configuration, Mapping, DC Clock
Configuration, topology-wide DC SYNC Configuration, and/or legacy DC
Configuration. The scheduler orders these services as PDO, Watchdog, Mapping,
DC Clock, DC SYNC, legacy DC, then Startup. It releases Startup only after the whole PDO
batch and other required controllers reach real Complete phases, then resumes
the retained topology through SAFEOP/OP. Logical addresses remain master-owned.
The full verified-bank mapping path rejects position/station/count/index drift
before actions, zeroes and reads back every discovered SyncManager slot
including unused slots, then every discovered FMMU slot, and finally runs the
existing desired SM/FMMU write/readback sequence. The legacy start and FMMU-only
start remain source-compatible. Observed register pages are evidence and reset
bounds, not desired product configuration. Physical response authenticity,
interoperability, target WCET and hardware qualification remain outside this
software claim.

The configuration SHA-256 covers normalized product semantics and a sorted
label-to-semantic-ESI-hash map. It excludes timestamps, host paths, compiler,
output directory, JSON key order, and XML formatting.

`robot_build_input.json` uses `esop.product-build-input.v1`. Its devices object
contains one exact Complete Access record and one exact SDO Information record
per declared slave with matching name/position, ESI support and product
enablement. An enabled SDO Information record also carries the strictly ordered,
deduplicated selected-PDO expectation list: owner position, object/subindex,
exact CANopen data type, bit length and required read/write/PDO-mappability flags.
The report boundary rejects length or identity mismatch, duplicate
name/position, non-boolean fields, enablement without support, disabled/nonempty
or enabled/empty plans, malformed hexadecimal values, unknown data types/access
flags, wrong ownership and duplicate/unordered expectations. Optional build
report projection copies the config hash, platform, devices,
PDO/frame/wire/WKC/copy metrics, cycle budget, and resources while preserving
`qualification.passed=false`. Default build-report generation without a
product input retains its host-placeholder shape.

Wire bytes use the ADR formula:

```text
wire_bytes = 8 + max(64, 14 + 2 + ethercat_datagram_bytes + 4) + 12
```

The terms are preamble/SFD, MAC header, EtherCAT frame header, generated
datagrams, FCS, and inter-packet gap respectively.

## 4. Validation & Error Matrix

| Condition | Required result |
| --- | --- |
| Unknown/missing JSON field or unsupported schema | Reject before staging. |
| Empty watchdog object, explicit zero, unknown watchdog field, duplicate plan position/station, or plan overflow | Reject before artifact publication or controller mutation. |
| Absolute, parent-traversing, or symlink-escaping ESI path | Reject as invalid product input. |
| Ambiguous ESI identity/PDO, duplicate object, wrong direction/width | Reject with identity/PDO/CiA 402 context. |
| Unknown, misplaced, empty, or over-capacity ESI FMMU declaration | Reject before staging or hash publication. |
| Nested/misplaced ESI DC OpMode or wrapped DC field | Reject before mode or device metadata publication. |
| Zero/malformed/overflowing ESM timeout or non-output OpOnly SM | Reject before staging or hash publication. |
| Missing CoE, partial/duplicate/disabled ESI mailbox SM, or invalid mailbox range | Reject before staging or hash publication. |
| Malformed ESI Complete Access/SDO Information capability, product enablement without support, or duplicate/malformed build-input policy evidence | Reject before staging, runtime mutation, or build-report publication. |
| Empty/over-capacity/conflicting generated SDO Information expectation, wrong runtime owner, invalid type/width/access, or duplicate/unordered plan | Reject before artifact publication, runtime activation, mailbox request, or build-report publication. |
| Malformed, out-of-order, over-capacity, wrong-service/opcode/object/subindex/value-info SDO Information response, or abort | Latch the typed transfer/verifier fault, clear pending publication, and never report a partially verified plan. |
| Duplicate Domain/slave/axis identity or overlapping range | Reject before registry mutation/publication. |
| Capacity, schedule, raw policy, or ProcBuf layout overflow | Reject with the owning contract error. |
| Generation failure with an existing output | Preserve the previous six-file directory byte-for-byte. |
| Successful regeneration | Replace the directory and remove stale schema files. |
| Runtime schema/hash/ProcBuf/topology mismatch | Reject before registry activation. |
| Invalid/misaligned startup profile or OpOnly without exclusive RxPDO | Reject before Startup mutation or control emission. |
| Invalid expected mailbox, live SII parse/CoE/layout mismatch, or SII request fault | Latch typed Startup fault before AL and publish no mailbox evidence. |
| SII category image exceeds capacity, lacks END, overflows EEPROM addressing, or fails a response check | Latch the first stream fault and publish neither image nor configuration candidate. |
| Invalid generated FMMU count/usage-to-PDO direction, SII SM mask/PDO grouping, empty per-slave PDO mapping, or live schema-v2 structural signature mismatch | Reject before Startup mutation or latch Startup before AL; publish no SII verification evidence. |
| Live SII FMMU usage or SyncManager count exceeds the matching ESC scan record count | Latch the typed Startup count fault before evidence or AL; publish no register, SII or DC evidence. |
| FMMU register count exceeds capacity, or a page has stale ownership, wrong WKC/length/generation, or timeout | Latch the typed discovery/Startup fault and publish no FMMU/SII/DC evidence. |
| SyncManager register count exceeds capacity, or a page has stale ownership, wrong WKC/length/generation, or timeout | Latch the typed discovery/Startup fault and publish no FMMU/SyncManager/SII/DC evidence. |
| Verified FMMU/SyncManager bank position, station, count or index differs from desired mapping, or a cleared/configured page reads back differently | Reject before the first mapping action or latch Mapping fault; never report the configuration Complete. |
| DC reference without required, or multiple generated references | Reject before Startup mutation or generated output publication. |
| Required System Time absent, delay-only WKC 0, or explicit reference not capable | Latch Startup before identity/SII/AL and publish no selected reference. |
| System Time WKC greater than one, malformed payload, stale generation, ownership mismatch, or timeout | Latch the first typed scan fault; publish no partial slave record. |
| Receive-time/Data Link Status WKC other than one, malformed payload, stale generation, ownership mismatch, or timeout | Latch the first typed scan fault; publish no partial slave record. |
| Duplicate/unreachable/overrun topology, missing DC receive times, invalid reference, delay underflow, or aggregate overflow | Latch typed topology failure before identity and publish neither reference nor topology. |
| Product-required DC slave has no measurable reference-relative delay | Latch `DcPropagationDelayRequired` before identity and publish neither reference nor topology. |
| Generated/raw DC timing mismatch, invalid factor/activation, plan/topology/reference mismatch, or common-epoch overflow | Reject before the first SYNC action or latch the first typed controller fault; publish no programmed evidence. |
| Watchdog action/generation/length/WKC/readback/deadline/control-pool mismatch | Latch the first typed Watchdog fault, publish no programmed evidence, and keep the PREOP barrier closed. |
| Runtime Domain/axis evidence or capacity mismatch | Reject with typed owning-contract evidence and return no partial configuration. |
| Slave-copy target is the motion Domain or is not backed by a shared auxiliary image | Reject owner construction with `InvalidCopyTarget`; do not arm production RX. |
| Slave-copy target cycle is not exactly the finalized RX cycle plus one, or was already published | Return `ScheduledSlaveCopyError::CycleOrder`; preserve the published page and fault the copy-enabled owner. |
| Referenced source/target Domain is missing, or a plan fails address/image preflight | Return the plan-indexed typed error before staging mutation; preserve the published page and fault the owner. |
| Shared auxiliary publication is absent, has the wrong cycle/length, or does not point to the exact safe image | Return `InvalidAuxiliaryOutputs` before frame acquisition, TX, State mutation, or event publication. |
| PDO plan owner/SM/group/capacity mismatch | Reject before returning any startup plan. |
| Invalid generated mailbox or invalid/missing/duplicate/unknown override binding | Reject before returning any batch. |
| Missing, duplicate, out-of-bounds or tampered mapped mailbox status, or inconsistent Domain/LRD/WKC coverage | Reject before product activation and publish no mapped binding. |
| Mapped mailbox status input is inactive, invalid or stale | Suppress the input-mailbox read, report bounded progress, and never issue a direct status-register fallback. |
| Duplicate batch station, insufficient jobs/operations, or generation overflow | Reject before replacing or starting a batch. |
| PDO upload readback length or byte mismatch | Latch controller fault and keep the current operation index. |
| PDO mailbox terminal failure | Latch the exact typed transport fault and keep the current operation index. |
| Substituted PDO mailbox/controller binding | Reject before TX without consuming or advancing the PDO action. |
| Product build input with unknown fields, invalid hash/budget, invalid Complete Access/SDO Information evidence, or `passed=true` | Reject before report write. |
| Missing target/HIL/WCET/resource evidence | Keep report unqualified. |

## 5. Good / Base / Bad Cases

- Good: the checked-in dual-drive plus IO example generates six artifacts, a
  C11-clean header, a byte-identical compiled Rust module, 42 process-image
  bytes, 2 frames, WKC 8, 4 maximum due copy bytes, 181 wire bytes, a
  4144-byte ProcBuf region,
  two DC-required drives, one explicit left-drive reference, Startup-owned
  measurable propagation-delay evidence for both required drives, and a
  two-entry 1 ms DC SYNC plan with exact `AssignActivate=0x0300`; both drives
  also carry divider 2500 and process-data interval 100 while the IO slave
  preserves ESC watchdog defaults. Both drives advertise Complete Access and
  SDO Information, but only the left drive is product-authorized for each; its
  seven selected PDO entries form a sorted typed verification plan. The right
  drive remains supported/disabled and the IO slave remains unsupported/disabled.
- Base: no `PRODUCT_INPUT` produces the existing unqualified host build
  report; an omitted slave `dc` object produces no Startup DC requirement, and
  optional unmeasurable DC evidence remains explicitly `None`; omitted CoE
  policy fields disable Complete Access and SDO Information with empty plans; an omitted
  `watchdog` object produces an empty watchdog plan and no ESC requests. A
  product with no slave-copy plans keeps the existing static auxiliary-output
  owner path and reports zero applied copies.
- Bad: a selected RxPDO moved to TxPDO, a malformed Controlword width, a
  duplicate object, a path escape, zero product limit, reference-without-
  required, duplicate reference, wrapped/misplaced DC mode field, malformed
  port tree, required unmeasurable DC, empty/zero watchdog declaration,
  watchdog readback mismatch, unsupported SDO Information enablement, wrong
  expectation owner/type/access/order, SDO Information abort or malformed
  fragment, motion-Domain copy target, replayed copy cycle,
  substituted publication page, or forged qualification fails without partial
  publication.

## 6. Tests Required

- Compare runtime ProcBuf descriptor sizes/hash with const-generic ABI types.
- Exercise `validate_for_product` zero limits and every raw overflow class.
- Prove byte-identical output under JSON key/XML whitespace changes.
- Cover identity, direction, width, duplicate object, range, capacity, path,
  string escaping, atomic failure preservation, and stale-file removal.
- Compile the generated header with
  `gcc -std=c11 -Wall -Wextra -Werror -x c -fsyntax-only`.
- Compare the generated Rust module byte-for-byte with the checked-in example,
  activate it in integration tests, and check `esop-product-config` for
  `aarch64-unknown-none`.
- Cover omitted/default DC policy, unknown fields, reference invariants,
  ESI multi-mode order, signed factor widths, nested/misplaced DC fields,
  configuration-hash changes, generated C/Rust/JSON/inventory fields, runtime
  profile propagation, 32/64-bit System Time reads, WKC 0/1/>1, malformed and
  timed-out responses, explicit/fallback selection, pre-identity mismatch and
  restart clearing. Cover exact four-port receive-time and Data Link Status
  reads, delay-only continuation, 3/1/2 linear and branched adjacency, wrapping
  timestamps, checked underflow/overflow, non-first reference traversal,
  optional incomplete evidence, required-unmeasurable rejection and
  transactional clearing. Include public master/control integration tests
  proving both WKC 0 and the 16-byte receive-time response traverse the normal
  RX ownership path.
- Cover direct and factor-derived DC cycles, signed shifts, SYNC1 relation,
  invalid activation, inexact division and arithmetic limits in the shared
  resolver. Prove generated JSON/inventory/C/Rust and configuration hashes carry
  resolved values, the runtime rebuilds an ordered product plan, and the
  all-slave controller uses one reference read, one LCM-aligned common epoch,
  exact register ordering, request ownership, complete-only publication,
  restart clearing and PREOP/lifecycle gating through the simulated Linux path.
- Cover omitted, divider-only, interval-only and dual-field watchdog input;
  unknown/empty/zero declarations; deterministic JSON/inventory/C/Rust/hash
  output; runtime product-order plan reconstruction; exact `0x0400/2` then
  `0x0420/2` write/read ordering; little-endian payloads; WKC/length/readback/
  action/generation/deadline/pool failures; complete-only publication,
  explicit restart, scheduler priority between PDO and Mapping, Linux simulated
  request/RX routing and PREOP/lifecycle gating.
- Cover explicit/default ESM timeouts, invalid values, ESI/SII OpOnly flag
  separation, PREOP-disabled mapping, enable-after-OP, disable-before-leaving,
  shared deadlines, uniform-override precedence, exact readback failure and
  generated startup-profile propagation.
- Build exact per-slave drive/IO PDO plans from the checked-in generated module;
  cover assignment-disable ordering, mapping grouping, all typed rejection
  paths, exact/segmented readback, mismatches, stale actions and restart.
- Build the checked-in drive/drive/IO batch from generated mailbox data;
  compare every station/config/plan with the individual plans, prove the
  explicit override path, and cover invalid generated config, missing,
  duplicate, unknown, station-duplicate and capacity failures.
- Prove mailbox address/capacity changes alter ESI semantic and configuration
  hashes while XML formatting-only changes remain byte-identical. Cover exact
  SII fixed-header direction conversion, Startup exact-range acquisition,
  generated-versus-live layout comparison, action ownership, timeout,
  multi-slave order, legacy opt-out and all shape/protocol/range failures.
- Cover deterministic per-Domain mailbox status tail packing, aggregate LRD
  length/WKC and hash propagation; reject Domain, bit, age, FMMU index,
  logical/physical address, direction, duplicate or coverage tampering. Pass
  the exact generated descriptor through Mapping write/readback, then prove
  mapped mailbox inactive/invalid/stale suppression, fresh recovery and no
  direct-register fallback while preserving PDO direct polling and PollTime.
- Cover bounded SII category-stream acquisition through END, unknown and
  zero-length categories, capacity/missing-END/address failures, non-reset
  action cursors, one absolute deadline, request-pool ownership, stale
  responses, signed projection and candidate atomicity. Cover supported FMMU
  values, order, duplicate/empty/unknown/capacity rejection, every schema-v2
  signature field, generated profile construction, PDO direction compatibility,
  ESC count gating, exact match/mismatch,
  configuration-action ownership, timeout, multi-slave reuse, restart clearing,
  and legacy opt-out before AL.
- Cover ordered FMMU then SyncManager register-page discovery, zero/over-capacity
  banks, exact address/WKC/length/generation/action ownership, control-pool
  completion, timeout/restart, atomic FMMU/SyncManager/SII/DC publication, and
  failure without partial evidence. Cover the Startup mapping bridge and both
  complete discovered-bank zero-write/readback sequences, including unused
  slots and position/station/count/index/readback failures, through the
  production scheduler/control-pool path.
- Drive the checked-in generated slave-copy plan through the stable production
  owner. Assert a non-due target publishes an unchanged page with zero
  applications, a fresh source produces the exact N+1 IO frame and quality 1,
  and bad WKC produces fallback plus quality 0. Assert all-plan preflight keeps
  the old page unchanged, replay faults the owner, motion targets are rejected,
  and release evidence reports the exact due-copy count.
- For shared auxiliary output, assert missing, cycle-zero, stale, wrong-length,
  and pointer-substituted typed snapshots fail before TX or lifecycle
  publication. Assert the exact owner-published snapshot is accepted and the
  static-image compatibility path still passes without a snapshot.
- Route a generated-style PDO action through `ScheduledPdoConfiguration`, the
  existing mailbox/DC/shared-RX path, exact upload readback, request rebuild,
  cross-generation waiting, timeout, lifecycle gating, fault blocking and
  explicit restart. Cover the opt-in PREOP Startup barrier, automatic release
  from actual Complete phases, retained topology and legal SAFEOP/OP
  progression. Cover two-job automatic advancement and whole-batch release;
  keep physical response authenticity and HIL outside this software claim.
- Validate both default and product-input build reports, including forged pass
  rejection and exact wire metric projection.
- Run `make ci`, `make bpf`, and `make test-zenoh` before delivery.

## 7. Wrong vs Correct

### Wrong

```rust
// Private arithmetic bypasses activation contracts and undercounts wire use.
let frame_bytes = 16 + pdo_bytes;
let expected_wkc = manifest.expected_wkc;
```

### Correct

```rust
// Register and activate through the runtime contracts, then derive evidence.
registry.register_pdo_at(domain_id, offset, request)?;
let schedule = registry.activate_with_frame_plans(period_ns, &mut plans)?;
let wire_bytes = 8 + mac_frame_bytes + 12;
```

Generated plans and exact comparison of supplied SDO responses are software
evidence, not proof of production mailbox execution, authentic physical
read-back, drive behavior, measured timing, or functional safety.

### Slave-copy publication ownership

Wrong:

```rust
// A raw slice does not prove which cycle or page produced these bytes.
context.safe_process_image = image.published();
context.shared_process_image = None;
owner.complete_receive(&bank, &received)?;
```

Correct:

```rust
let publication = owner.complete_receive_with_slave_copies(
    &bank,
    &received,
    &mut image,
)?;
context.safe_process_image = image.published();
context.shared_process_image = Some(image.published_image());
assert_eq!(publication.target_cycle(), received.report.cycle + 1);
```

The typed snapshot is still ordinary software evidence. It proves the local
owner/page/cycle handoff, not physical slave execution or functional safety.

### DC policy and scan evidence

Wrong:

```rust
// Slave kind is not proof of DC capability, and direct selection bypasses scan.
let reference_station = product.slaves[0].station_address;
```

Correct:

```rust
let profiles = product.startup_profiles()?;
product.start_startup(&mut startup, generation, now_ns, config, observed)?;
let reference = startup.selected_reference_clock(); // Published after scan validation.
let topology = startup.dc_topology(); // Published in the same transaction.
dc_clock.start(
    dc_clock_config,
    topology.ok_or(Error::MissingTopology)?,
    generation,
    application_time_ns,
    monotonic_now_ns,
)?;
```

For ESI parsing, checking only the local element name is also wrong because a
wrapped DC `Name` can otherwise be reinterpreted as the Device name. Keep an
explicit DC builder and require the complete direct-child path before assigning
any mode field; reject malformed nesting before the generic name handlers run.

Do not treat a missing propagation delay as zero or program ESC delay/offset
registers from an unpublished candidate. The only zero-delay exception is the
selected reference already published with measured topology evidence. Optional
evidence stays `None`; a product-required slave without a measured path must
fail before identity, and a partial physical write sequence must never be
reported as a complete software batch.
