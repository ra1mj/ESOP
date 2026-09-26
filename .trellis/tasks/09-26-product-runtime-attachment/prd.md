# Generated Product Runtime Attachment

## Goal

Close the software-side gap between the deterministic `esop-cfggen` product
artifacts and the production-shaped `no_std` runtime. A generated product
configuration shall compile into firmware as static Rust data and activate
only after the observed EtherCAT topology, Domain/PDO/datagram plan, ProcBuf
ABI, schedule, CiA 402 PDO maps, and per-axis command policies all match the
frozen product contract.

## Background

`esop-cfggen` already validates a strict product manifest and selected ESI
files through the existing runtime planning APIs. It emits deterministic C and
JSON artifacts plus a configuration hash, but production firmware cannot yet
consume that evidence directly. The current runtime APIs can validate each
component independently, while the missing attachment layer must combine them
transactionally and fail before any product configuration becomes active.

## Functional Requirements

1. Add a dedicated `no_std` crate that defines the static generated product
   contract and owns runtime activation. It must not allocate or parse JSON,
   XML, or C artifacts.
2. Extend `esop-cfggen` with a deterministic `esop_product_config.rs` artifact
   containing the complete static slave, Domain, PDO, datagram, axis, policy,
   ProcBuf layout, schema, product identity, and configuration-hash data.
3. The generated Rust module must depend only on the runtime attachment crate's
   public/re-exported API, compile in `no_std` firmware, and avoid host paths,
   timestamps, mutable global state, and dynamic initialization.
4. Runtime activation must validate the generated schema version and caller's
   expected 32-byte configuration hash before validating any mutable runtime
   object.
5. Runtime activation must require an exact observed slave set. It shall reject
   missing, extra, offline, unconfigured, duplicate-position, position,
   station-address, vendor, product-code, revision, and non-wildcard serial
   mismatches.
6. Runtime activation must recompute the const-generic ProcBuf layout and
   validate the live ProcBuf header for magic, ABI version, layout hash, robot
   ID, boot ID, region size, and axis/IO/Domain/event capacities.
7. Runtime activation must rebuild the frozen Domain/PDO/datagram plan through
   the existing `DomainRegistry` registration APIs and activate schedule/frame
   plans through the existing activation API. Private approximations are not
   acceptable.
8. The rebuilt plan must match each generated Domain's expected PDO/datagram
   counts and expected total/input WKC. Registration, frame, schedule, and
   capacity failures must retain their existing typed evidence.
9. Each generated axis must use a unique contiguous index, own a configured
   CiA 402 drive, contain a policy accepted by
   `Cia402AxisCommandPolicy::validate_for_product`, and derive a
   `Cia402PdoMap` valid for its selected CSP, CSV, or CST mode from the frozen
   PDO registrations.
10. Successful activation must return an owning, immutable activation result
    containing the validated registry, schedule, frame plans, product metadata,
    axis modes, policies, and PDO maps. Failed activation must not partially
    publish caller-visible product state.
11. Check in the expected generated Rust module for the dual-axis simulator
    product and compile/use it in integration tests. Generation must reproduce
    that file byte-for-byte.
12. Add Makefile and GitHub Actions coverage that regenerates the example,
    compares the Rust module with the checked-in golden file, compiles/activates
    it, and checks the new runtime crate for `aarch64-unknown-none`.

## Safety and Determinism Requirements

1. All activated cyclic data must have fixed capacity known at compile time.
   The attachment layer may use a documented fixed upper bound for temporary
   per-axis PDO-map construction and must reject products that exceed it.
2. Generated metadata, arrays, identifiers, float literals, hashes, and enums
   must be rendered deterministically and as valid Rust syntax. String content
   must be escaped through Rust literal semantics.
3. The configuration hash embedded in Rust must exactly equal the canonical
   hash in `product_config.json`, `esop_product_config.h`, and
   `robot_build_input.json`.
4. No topology identity, Domain evidence, ProcBuf capacity, operating mode,
   axis policy, or required CiA 402 object may silently default during runtime
   activation.
5. Activation failures must be typed, deterministic, and identify the relevant
   slave, Domain, axis, or capacity where practical.
6. Generated simulator activation is development evidence only. It must not
   claim real-slave interoperability, measured wire timing, target WCET or
   resource qualification, HIL completion, mechanical suitability, braking
   verification, FSoE, or functional-safety qualification.

## Compatibility Requirements

1. Existing cfggen JSON/C outputs and consumers remain compatible; the Rust
   module is an additive sixth artifact.
2. ProcBuf ABI remains version 6 and existing runtime crates remain `no_std`.
3. Existing products may continue to use prior hand-wired runtime construction,
   but the checked-in simulator example and documentation shall demonstrate the
   generated attachment path.
4. The initial runtime attachment validates the generated identity and static
   plan. Live SII/PDO assignment read-back beyond the supplied observed slave
   records remains a separate hardware-integration responsibility.

## Acceptance Criteria

- [x] `esop-cfggen` emits six deterministic artifacts, including a valid
      `esop_product_config.rs` with the exact canonical configuration hash.
- [x] A new `no_std` runtime crate activates the checked-in dual-axis simulator
      configuration with three slaves, two Domains, two axes, schedule
      hyperperiod four, and two Domain frame plans.
- [x] Integration tests prove exact metadata/hash/layout and the activated
      registry, schedule, frame-plan, mode, policy, and PDO-map evidence.
- [x] Tests reject schema/hash mismatch, ProcBuf robot/boot/layout/capacity
      mismatch, missing/extra/offline/unconfigured/wrong-identity topology,
      invalid Domain evidence or capacity, and invalid axis
      ownership/index/mode/policy/map.
- [x] Reordered manifest keys and reformatted ESI inputs still produce the same
      configuration hash and byte-identical Rust artifact.
- [x] The example Rust module is reproduced and compared byte-for-byte in CI,
      then compiled and exercised through runtime activation.
- [x] `cargo check -p esop-product-config --target aarch64-unknown-none` passes.
- [x] README, software PRD, robotics plan, product-configuration documentation,
      capability manifest, and Trellis backend specification describe the
      implemented runtime attachment and remaining physical qualification gaps.
- [x] `make ci`, BPF build, Zenoh tests, formatting, Clippy, release/no_std
      checks, and `git diff --check` pass locally.
- [ ] Changes are committed, pushed to `ra1mj/ESOP`, and the corresponding
      GitHub Actions run succeeds.

## Out of Scope

- Reading ESI/SII or assigning PDOs dynamically on the target.
- Proving a physical slave accepted the generated mapping or station address.
- Target-specific WCET, CPU/memory resource qualification, physical wire-time
  measurements, HIL, servo tuning, braking/mechanical verification, FSoE, or
  functional-safety certification.
- Replacing the existing lifecycle safety, topology, health, watchdog, or
  eBPF observability mechanisms.
