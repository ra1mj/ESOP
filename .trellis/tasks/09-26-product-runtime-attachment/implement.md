# Implementation Plan: Generated Product Runtime Attachment

## 1. Add the Runtime Contract Crate

- [x] Add `esop-product-config` to the workspace with only `no_std` runtime
      dependencies.
- [x] Define metadata, slave, Domain, PDO, datagram, axis, static product, and
      activated product types.
- [x] Re-export the exact dependency types needed by generated Rust modules.
- [x] Define typed activation errors and immutable result accessors.

Validation:

```bash
cargo check -p esop-product-config
cargo check -p esop-product-config --target aarch64-unknown-none
```

## 2. Implement Transactional Activation

- [x] Validate schema, expected configuration hash, runtime layout descriptor,
      and complete live ProcBuf header.
- [x] Validate exact observed slave topology and identity.
- [x] Rebuild Domains, PDO registrations, and datagrams through existing APIs.
- [x] Compare generated Domain evidence and activate schedules/frame plans.
- [x] Validate contiguous axes, drive ownership, policies, and selected-mode
      CiA 402 PDO maps with bounded scratch storage.
- [x] Return all frozen evidence only after every stage succeeds.
- [x] Add focused unit tests for each fail-closed validation class.

Validation:

```bash
cargo test -p esop-product-config --lib
```

Rollback point: the additive crate can be removed without changing callers.

## 3. Emit the Deterministic Rust Artifact

- [x] Add `esop_product_config.rs` to cfggen's atomically published artifact
      map.
- [x] Render metadata, exact arrays/slices, enum values, policies, ProcBuf
      descriptor, and decoded configuration hash as valid Rust.
- [x] Enforce the runtime crate's per-axis PDO scratch limit during generation.
- [x] Update artifact-count and deterministic-output tests from five to six.
- [x] Add renderer tests for escaping and stable value syntax.

Validation:

```bash
cargo test -p esop-cfggen
```

## 4. Add the Compiled Golden Product

- [x] Generate and check in the dual-axis simulator Rust module.
- [x] Include the module in a runtime integration test and activate it against
      matching observed slaves and a live ProcBuf.
- [x] Assert exact hash/layout, three slaves, two Domains, two axes,
      hyperperiod four, and two frame plans.
- [x] Add generated-module mutation tests for schema/hash/topology/layout/
      Domain/axis rejection without editing the golden artifact.

Validation:

```bash
cargo run -p esop-cfggen -- \
  --input config/examples/sim-dual-axis/product.json \
  --output build/generated/sim-dual-axis
cargo test -p esop-product-config
```

## 5. Integrate Build and CI Gates

- [x] Add `cfggen-runtime-example` to regenerate and `cmp` the checked-in Rust
      artifact before executing the runtime tests.
- [x] Make cfggen/CI targets depend on the runtime example gate.
- [x] Include the new crate in the no_std target checks.
- [x] Preserve generated-header compilation and build-report generation.

Validation:

```bash
make cfggen-runtime-example
make no-std
make ci
```

## 6. Update Product Contracts

- [x] Update README, software PRD, robotics plan, and product-configuration
      documentation from five to six artifacts and describe runtime activation.
- [x] Add the runtime attachment capability and revise stale limitations.
- [x] Extend the Trellis product-configuration specification with generated
      Rust and activation invariants.
- [x] Keep physical/HIL/WCET/resource/safety qualification gaps explicit.

## 7. Quality Gate and Delivery

- [x] Run formatting, workspace tests, Clippy, release/no_std checks, cfggen
      reproduction, BPF build, Zenoh tests, and `git diff --check`.
- [ ] Complete task acceptance and record the Trellis journal.
- [ ] Commit and push to `origin/main`.
- [ ] Monitor GitHub Actions to success and repair any failure.

Validation:

```bash
make ci
make bpf CLANG="$HOME/.local/opt/clang14/usr/bin/clang-14"
make test-zenoh
git diff --check
```
