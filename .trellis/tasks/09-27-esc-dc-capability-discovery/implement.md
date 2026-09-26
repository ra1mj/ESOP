# Implementation Plan

## 1. Correct scan base evidence

- [x] Add Features Supported register/bit constants and explicit DC range.
- [x] Correct `ScanRecord` protocol widths and decode the bounded base block.
- [x] Add exact byte-pattern regression tests for every field.

Validation:

```bash
cargo test -p esop-ethercat-core scan
```

## 2. Probe System Time capability

- [x] Add the DC capability scan phase and zero-or-one WKC contract.
- [x] Decode 32/64-bit System Time samples without partial publication.
- [x] Cover non-DC skip, WKC 0/1/>1, malformed payload, timeout, stale
      generation, and restart.

Validation:

```bash
cargo test -p esop-ethercat-core scan
cargo test -p esop-ethercat-core startup
```

## 3. Bind Startup policy and reference selection

- [x] Add `StartupDcRequirement` and profile propagation.
- [x] Validate all profiles transactionally before identity/SII/AL actions.
- [x] Publish explicit or first-capable reference station/position evidence.
- [x] Update simulator request routing and multi-slave integration tests.

Validation:

```bash
cargo test -p esop-ethercat-core startup
cargo test -p esop-ethercat-linux-port --test scheduled_domains
```

## 4. Generate product DC policy

- [x] Extend the strict product schema with optional per-slave DC policy.
- [x] Validate reference invariants and carry policy through normalized JSON,
      C, Rust, hashes, and runtime profiles.
- [x] Update the dual-axis simulator product and deterministic artifacts.
- [x] Add invalid-policy, hash-change, deterministic-output, and generated
      runtime profile tests.

Validation:

```bash
cargo test -p esop-cfggen
cargo test -p esop-product-config
make cfggen-runtime-example
git diff --exit-code -- config/examples/sim-dual-axis/expected
```

## 5. Cross-layer documentation and qualification

- [x] Update PRD/requirements, product contract, capability manifest, README,
      and backend spec with the exact software boundary.
- [x] Preserve explicit propagation-delay, synchronization, authenticity,
      timing, HIL, conformance, and safety limitations.
- [x] Run focused checks, no-std, generated-artifact gates, and complete CI.
- [x] Commit, push `main`, and wait for the exact GitHub Actions run.

Validation:

```bash
cargo fmt --all -- --check
git diff --check
make no-std
make capability-manifest
make ci
```

## Review Gates

- No identity/SII/AL action is observable after a required DC mismatch.
- WKC 0 is capability-negative only for the System Time probe.
- Reference selection never uses a delay-only or non-DC slave.
- Invalid product policy cannot publish a partial generated directory.
- The cyclic path gains no allocation, blocking, sleeping, or logging.
- Documentation does not imply complete DC topology/configuration or HIL.

## Risk And Rollback Points

- `ScanRecord` field-width correction is a public source change; update all
  fixtures and consumers together and reject compatibility shims that preserve
  wrong values.
- Scan sequencing changes every Startup simulation. Centralize response
  helpers and route by register/phase instead of relying on action ordinal.
- Generated schema/artifacts/runtime types form one contract and must be
  committed atomically.
