# Design: Generated Product Runtime Attachment

## Architecture

Add `crates/esop-product-config` as an allocation-free runtime adapter between
generated static product data and the existing EtherCAT, ProcBuf, lifecycle,
and CiA 402 contracts.

```text
product.json + selected ESI
          |
          v
      esop-cfggen
          |
          +--> existing JSON/C/build evidence
          +--> esop_product_config.rs
                         |
                         v
              StaticProductConfig<...>
                         |
       observed slaves + live ProcBuf + expected hash/boot ID
                         |
                         v
                 activate_product
                         |
        +----------------+----------------+
        |                |                |
  ProcBuf/header   exact topology   rebuilt Domain plan
                                      + schedule/frames
                                             |
                                  axis policy/PDO-map checks
                                             |
                                             v
                                  ActivatedProduct<...>
```

## Static Contract

The runtime crate owns compact generated structs:

- `ProductMetadata`: generated schema, product name, configuration SHA-256,
  robot ID, policy version, base period, and deadline;
- `ProductSlaveConfig`: name, position, station address, Domain, kind, and
  `SlaveIdentity`;
- `ProductDomainConfig`: name, `DomainConfig`, expected PDO/datagram counts,
  expected WKC, and expected input WKC;
- `ProductPdoConfig`: Domain and owning slave position plus the exact
  `PdoRegistrationRequest`;
- `ProductDatagramConfig`: Domain plus `DomainDatagramSpec`;
- `ProductAxisConfig`: axis name/index, owning drive position, selected
  `OperatingMode`, and validated `Cia402AxisCommandPolicy`;
- `StaticProductConfig<SLAVES, DOMAINS, AXES>`: metadata, exact slave/Domain/
  axis arrays, static PDO/datagram slices, and `ProcBufLayoutDescriptor`.

The crate re-exports the dependency types required by generated modules so a
generated artifact has one stable import surface. Generated values are pure
`const`/`static` data and do not require a runtime initializer.

## Activation API

`StaticProductConfig::activate` is generic over live ProcBuf capacities and
runtime registry/schedule/frame limits. Inputs are:

- the caller-expected configuration hash;
- observed `SlaveRecord` values collected by the existing topology path;
- a reference to the live `ProcBuf`;
- the current boot ID.

Activation builds all derived state into local values and returns
`ActivatedProduct` only after every stage succeeds. The result owns its
`DomainRegistry`, `ScheduleTable`, `FramePlanSet`, and fixed axis mode/policy/
PDO-map arrays. Read-only accessors expose evidence without allowing the frozen
plan to be mutated through the attachment API.

## Validation Order

1. Check generated schema and caller-expected configuration hash.
2. Recompute `describe_layout` for the live const-generic dimensions, compare
   it with the generated descriptor, and validate the ProcBuf header including
   product robot ID and boot ID.
3. Require exact observed/generated slave cardinality, unique observed
   positions, online/configured state, station address, and
   `SlaveIdentity::matches`.
4. Rebuild Domains, PDO registrations, and datagrams through `DomainRegistry`.
5. Compare rebuilt per-Domain PDO/datagram/WKC evidence with generated values,
   then call `activate_with_frame_plans`.
6. Require contiguous unique axis indices, CiA 402 drive ownership, valid
   product policy, and a selected-mode `Cia402PdoMap` derived from registrations
   belonging to that drive.

This order rejects immutable contract mismatches before spending work on later
stages and never writes a caller-owned activation result on failure.

## Bounded PDO Map Construction

Rust stable const generics cannot express an array length derived from
`DOMAINS * PDO_CAPACITY`. The adapter therefore uses a documented fixed
`MAX_PRODUCT_AXIS_PDOS` scratch array of `PdoEntry` on the activation stack.
The generator enforces the same per-axis limit and activation returns a typed
capacity error rather than truncating entries.

## Error Model

`ProductActivationError` is a non-allocating typed enum. Variants preserve
underlying `DomainRegistrationError`, `ActivationError`, `FramePlanError`,
`ProcBufHeaderError`, policy, and CiA 402 PDO-map errors while adding indexed
context for schema/hash/layout, topology, generated evidence, ownership, and
fixed-capacity mismatches. It derives `Debug`, `Clone`, `Copy`, `Eq`, and
`PartialEq` when supported by wrapped types.

## Generated Rust Artifact

`esop-cfggen` renders `esop_product_config.rs` from the already validated
normalized product model. It emits:

- one `PRODUCT_CONFIG` static with exact const-generic cardinalities;
- static PDO and datagram arrays;
- enum constructors and integer/floating-point values with explicit types;
- a 32-byte configuration hash decoded from the canonical hexadecimal hash;
- Rust string literals rendered with debug escaping.

The artifact is inserted into the same `BTreeMap` as the five existing files,
so current staging-directory replacement preserves atomicity and stale-file
removal. Determinism tests compare all six artifacts.

## Example and Verification

The simulator example gains a checked-in expected Rust module. An integration
test includes it directly, constructs matching observed slaves and a live
`ProcBuf<2, 16, 2, 64>`, activates the product, and checks the exact registry,
schedule, frame-plan, axis-policy, and PDO-map evidence.

`make cfggen-runtime-example` regenerates the product, compares the module with
the golden file, and runs the runtime crate tests. `make cfggen-example` and
`make ci` depend on this path. `make no-std` checks the crate for
`aarch64-unknown-none`.

## Documentation and Qualification Boundary

The capability manifest gains a runtime-attachment capability and removes the
old statement that firmware cannot consume generated product configuration.
Limitations continue to state that live SII/PDO assignment read-back, physical
interoperability, measured wire timing, target WCET/resources, HIL, mechanical
braking, and functional safety remain unqualified.

## Rollback

The new crate and sixth artifact are additive. Existing JSON/C outputs and
hand-wired runtime construction continue to work. If attachment rollout must
be reverted, remove the workspace member, Rust renderer, golden test, and
documentation capability without changing ProcBuf ABI or existing cyclic code.
