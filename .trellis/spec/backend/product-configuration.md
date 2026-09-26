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
pub fn StaticProductConfig::start_startup(...) -> Result<(), ProductStartupError>;
```

## 3. Contracts

Input schema `esop.product.v1` is strict (`deny_unknown_fields`) and owns:

- product/robot/policy identity;
- ProcBuf dimensions and generator capacities;
- period/deadline and platform metadata;
- ordered Domain, slave, selected ESI PDO, and axis policy declarations.

ESI paths are confined relative paths and their canonical targets must remain
under the product directory. Names/labels rendered into C are nonempty,
NUL-free, and at most 128 UTF-8 bytes.

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
the caller-expected 32-byte hash, exact ProcBuf v6 descriptor/header, exact
observed online/configured topology, rebuilt Domain/PDO/datagram/WKC evidence,
schedule/frame plans, drive ownership, product policies, and selected-mode
CiA 402 PDO maps.
It returns the owning frozen result only after all checks pass.

Each generated slave carries four ESM transition timeout classes, one validated
CoE mailbox pair, and one bounded `OpOnlySyncManagerProfile`. ESI `MBoxOut`
maps to master-send/slave-receive and `MBoxIn` maps to
master-receive/slave-send. Both SyncManagers must be enabled and provide
`StartAddress`, `DefaultSize`, and `ControlByte`, while the Device must declare
`Mailbox/CoE`. Missing, duplicate, overlapping, overflowing, undersized, or
oversized mailbox ranges fail before publication. Timeout values are decimal
milliseconds converted to checked nanoseconds; missing values use the named
`ETG1020_DEFAULT_TRANSITION_TIMEOUTS_V1`. Mailbox data, timeout profiles, and
activation templates participate in ESI semantic and configuration hashes.

The core SII parser accepts only the exact five-word standard mailbox header,
checks CoE support and the same address/capacity rules, and performs the same
slave-to-master direction conversion. It parses caller-owned words or a
completed exact-range `SiiBlockReader`. A Startup profile may carry the
generated expected mailbox; after identity verification and before any AL
transition, Startup reads exactly SII words `0x001C..0x0020`, requires CoE,
compares only the four physical address/capacity fields, and publishes
position-keyed verified evidence. Runtime poll, timeout, retry and status-bit
policy are not SII layout fields. Profiles without an expected mailbox and the
legacy `start` API retain the identity-to-AL path.

The core also provides a separate fixed-capacity SII category stream contract.
`SiiCategoryStreamReader<WORDS>` starts at standard word `0x0040` by default,
reuses `SiiBlockReader` register transactions, and reads two-word headers plus
their declared payloads until `SII_CATEGORY_END`. Internal continuations keep
one absolute scan deadline and do not reset action-token or datagram-index
cursors. The image, category count, and byte projection stay unavailable until
END is accepted. Capacity, missing-END, address, WKC, generation, payload and
timeout failures are typed and latch the first terminal error. The companion
`SiiStreamDiscoveryController` publishes `SiiConfigurationCandidate` only
after caller-owned scratch conversion and complete transactional SM/RxPDO/
TxPDO projection; explicit PDO signedness is preserved. This contract is not
yet owned by Startup and does not compare the candidate with generated product
configuration.

`startup_profiles` validates timeout values, exact slave positions, generated
mailbox ranges, OpOnly flags, and exclusive selected RxPDO ownership before
returning fixed-array profiles. `start_startup` supplies those profiles to
Startup. A nonzero legacy
`StartupConfig.transition_timeout_ns` is an explicit uniform override;
otherwise each AL step selects its generated/default timeout. OpOnly and AL
work for one step share one absolute deadline. Non-OP and leaving-OP paths
disable/read back every OpOnly output before readiness or AL transition;
entering OP enables/read backs only after OP is observed. Any mismatch, WKC,
generation, length, or timeout fault blocks Ready.

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
`StartupConfig` into a PREOP barrier for PDO Configuration, Mapping, and/or DC
Configuration. The scheduler releases Startup only after the whole PDO batch
and other required controllers reach real Complete phases, then resumes the
retained topology through SAFEOP/OP. Full mapping/DC descriptor discovery,
physical response authenticity and hardware qualification remain caller work.

The configuration SHA-256 covers normalized product semantics and a sorted
label-to-semantic-ESI-hash map. It excludes timestamps, host paths, compiler,
output directory, JSON key order, and XML formatting.

`robot_build_input.json` uses `esop.product-build-input.v1`. Optional build
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
| Absolute, parent-traversing, or symlink-escaping ESI path | Reject as invalid product input. |
| Ambiguous ESI identity/PDO, duplicate object, wrong direction/width | Reject with identity/PDO/CiA 402 context. |
| Zero/malformed/overflowing ESM timeout or non-output OpOnly SM | Reject before staging or hash publication. |
| Missing CoE, partial/duplicate/disabled ESI mailbox SM, or invalid mailbox range | Reject before staging or hash publication. |
| Duplicate Domain/slave/axis identity or overlapping range | Reject before registry mutation/publication. |
| Capacity, schedule, raw policy, or ProcBuf layout overflow | Reject with the owning contract error. |
| Generation failure with an existing output | Preserve the previous six-file directory byte-for-byte. |
| Successful regeneration | Replace the directory and remove stale schema files. |
| Runtime schema/hash/ProcBuf/topology mismatch | Reject before registry activation. |
| Invalid/misaligned startup profile or OpOnly without exclusive RxPDO | Reject before Startup mutation or control emission. |
| Invalid expected mailbox, live SII parse/CoE/layout mismatch, or SII request fault | Latch typed Startup fault before AL and publish no mailbox evidence. |
| SII category image exceeds capacity, lacks END, overflows EEPROM addressing, or fails a response check | Latch the first stream fault and publish neither image nor configuration candidate. |
| Runtime Domain/axis evidence or capacity mismatch | Reject with typed owning-contract evidence and return no partial configuration. |
| PDO plan owner/SM/group/capacity mismatch | Reject before returning any startup plan. |
| Invalid generated mailbox or invalid/missing/duplicate/unknown override binding | Reject before returning any batch. |
| Duplicate batch station, insufficient jobs/operations, or generation overflow | Reject before replacing or starting a batch. |
| PDO upload readback length or byte mismatch | Latch controller fault and keep the current operation index. |
| PDO mailbox terminal failure | Latch the exact typed transport fault and keep the current operation index. |
| Substituted PDO mailbox/controller binding | Reject before TX without consuming or advancing the PDO action. |
| Product build input with unknown fields, invalid hash/budget, or `passed=true` | Reject before report write. |
| Missing target/HIL/WCET/resource evidence | Keep report unqualified. |

## 5. Good / Base / Bad Cases

- Good: the checked-in dual-drive plus IO example generates six artifacts, a
  C11-clean header, a byte-identical compiled Rust module, 36 PDO bytes, 2
  frames, WKC 6, 20 copy bytes, 180 wire bytes, and a 4144-byte ProcBuf region.
- Base: no `PRODUCT_INPUT` produces the existing unqualified host build
  report.
- Bad: a selected RxPDO moved to TxPDO, a malformed Controlword width, a
  duplicate object, a path escape, zero product limit, or forged qualification
  fails without partial publication.

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
- Cover bounded SII category-stream acquisition through END, unknown and
  zero-length categories, capacity/missing-END/address failures, non-reset
  action cursors, one absolute deadline, request-pool ownership, stale
  responses, signed projection and candidate atomicity.
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
