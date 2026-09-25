# Implementation Plan: Deterministic Product Configuration Generator

## 1. Add Shared Runtime Validation APIs

- [x] Add `ProcBufLayoutDescriptor` and runtime layout/hash calculation.
- [x] Prove descriptor sizes and hashes against const-generic ProcBuf types.
- [x] Expose reusable `Cia402AxisCommandPolicy` validation without changing
      cyclic behavior.
- [x] Add focused no_std crate tests for both APIs.

Validation:

```bash
cargo test -p esop-procbuf -p esop-lifecycle-guard
```

Rollback point: the additive helpers can be reverted independently of cfggen.

## 2. Implement Strict Product and ESI Parsing

- [x] Add host-only `esop-cfggen` workspace binary and dependencies.
- [x] Define strict `esop.product.v1` serde models and numeric ID decoders.
- [x] Implement namespace-tolerant ESI subset parsing with typed errors.
- [x] Add fixtures for dual CiA 402 drives and EtherCAT IO.
- [x] Test unknown fields, invalid identities, duplicate entries, and malformed
      XML/numeric values.

Validation:

```bash
cargo test -p esop-cfggen
```

## 3. Build and Validate the Frozen Product Plan

- [x] Normalize unique product/slave/Domain/axis identities.
- [x] Allocate stable byte-aligned Rx-then-Tx PDO offsets per Domain.
- [x] Register PDO/datagram plans through `DomainRegistry`.
- [x] Activate frame plans and multi-rate schedule within declared capacities.
- [x] Validate selected drive maps through `Cia402PdoMap`.
- [x] Validate axis masks and frozen SI/raw policies.
- [x] Derive WKC, frame, wire, copy, process-image, and cycle metrics.

Validation:

```bash
cargo test -p esop-cfggen --test generation
```

Rollback point: parsing remains useful even if plan generation is disabled.

## 4. Render and Publish Artifacts

- [x] Render normalized product JSON and device inventory.
- [x] Render exact ProcBuf layout and robot build input.
- [x] Render bounded static C structs/arrays and escape identifiers/strings.
- [x] Implement canonical hashing and byte-stable JSON/header output.
- [x] Implement atomic directory replacement and stale-file removal.
- [x] Compile the generated header with strict GCC warnings.
- [x] Test deterministic hashes/output and failure preservation.

Validation:

```bash
cargo run -p esop-cfggen -- \
  --input config/examples/sim-dual-axis/product.json \
  --output build/generated/sim-dual-axis
gcc -std=c11 -Wall -Wextra -Werror -x c -fsyntax-only \
  build/generated/sim-dual-axis/esop_product_config.h
```

## 5. Integrate Build Evidence

- [x] Add optional generated product input to build-report generation.
- [x] Validate generated schema/hash/metrics before copying fields.
- [x] Preserve default host report and fail-closed qualification behavior.
- [x] Add build-report tests for generated and malformed inputs.
- [x] Add Makefile targets and CI artifact generation/validation.

Validation:

```bash
make cfggen-example
make build-report PRODUCT_INPUT=build/generated/sim-dual-axis/robot_build_input.json
python3 -m unittest discover -s scripts/tests -p 'test_robot_build_report.py'
```

## 6. Update Product Contracts

- [x] Update README, software PRD, robotics plan, and add cfggen documentation.
- [x] Add `product_config_generator` capability and update related limitations.
- [x] Record the config-hash/layout/report contract in Trellis backend specs.
- [x] Keep explicit ESI subset, physical HIL, WCET, and safety limitations.

## 7. Quality Gate and Delivery

- [x] Run formatting, workspace tests, Clippy, release/no_std checks, and diff
      validation.
- [x] Run cfggen example reproduction and generated-header compilation.
- [x] Run BPF/eBPF and Zenoh integration checks available on the host.
- [x] Review generated artifacts and source diff for host-path/time leakage.
- [x] Complete task acceptance and Trellis journal.
- [x] Commit and push to `origin/main`.
- [x] Monitor GitHub Actions to success and fix/repush any failure.

Validation:

```bash
make ci
make bpf CLANG="$HOME/.local/opt/clang14/usr/bin/clang-14"
make test-zenoh
git diff --check
```
