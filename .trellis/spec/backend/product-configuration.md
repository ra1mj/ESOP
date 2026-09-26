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

Per-slave PDO startup-plan construction uses the same generated order and the
shared 256-entry cfggen bound. For each SyncManager it clears assignment
subindex zero, writes each mapping object, then publishes the ordered mapping
indexes and final assignment count. `PdoConfigController` executes each plan
write as download plus exact upload readback; operation progress is published
only after length and bytes match. The caller still owns production scheduling,
mailbox transport, retry policy, and CONFIGURING lifecycle admission.

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
| Duplicate Domain/slave/axis identity or overlapping range | Reject before registry mutation/publication. |
| Capacity, schedule, raw policy, or ProcBuf layout overflow | Reject with the owning contract error. |
| Generation failure with an existing output | Preserve the previous six-file directory byte-for-byte. |
| Successful regeneration | Replace the directory and remove stale schema files. |
| Runtime schema/hash/ProcBuf/topology mismatch | Reject before registry activation. |
| Runtime Domain/axis evidence or capacity mismatch | Reject with typed owning-contract evidence and return no partial configuration. |
| PDO plan owner/SM/group/capacity mismatch | Reject before returning any startup plan. |
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
- Build exact per-slave drive/IO PDO plans from the checked-in generated module;
  cover assignment-disable ordering, mapping grouping, all typed rejection
  paths, exact/segmented readback, mismatches, stale actions and restart.
- Route a generated-style PDO action through `ScheduledPdoConfiguration`, the
  existing mailbox/DC/shared-RX path, exact upload readback, request rebuild,
  cross-generation waiting, timeout, lifecycle gating, fault blocking and
  explicit restart. Keep PREOP orchestration and physical HIL outside this
  software claim.
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
