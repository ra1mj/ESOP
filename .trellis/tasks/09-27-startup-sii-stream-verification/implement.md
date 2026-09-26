# Implementation Plan

## 1. Core evidence model

- [x] Add the no-std SHA-256 dependency and versioned signature/builder types.
- [x] Extend `SiiProcessDataSegment` with PDO identity and entry range evidence.
- [x] Build signatures from candidates and add exact/mismatch regression tests.
- [x] Export new public types and fixed Startup capacity constants.

Validation:

```bash
cargo test -p esop-ethercat-core sii_config
cargo test -p esop-ethercat-core sii_discovery
```

## 2. Startup integration

- [x] Add profile expectation, configuration timeout, phase/action/progress,
      reusable discovery workspace, scratch storage, and verified evidence.
- [x] Route next/accept/timeout/control-pool ownership for the new action.
- [x] Gate AL after identity and optional mailbox verification.
- [x] Cover exact match, every mismatch class, stream faults, timeout,
      cross-action rejection, restart clearing, legacy opt-out, and multi-slave
      order.

Validation:

```bash
cargo test -p esop-ethercat-core startup
cargo test -p esop-ethercat-core production_service
```

## 3. Product configuration and generator

- [x] Add generated per-slave SM count/enabled-mask fields to Rust/C/JSON.
- [x] Build and validate signatures in `startup_profiles()` with typed errors.
- [x] Regenerate checked-in simulator artifacts and extend deterministic/hash,
      C compile, and generated-product tests.
- [x] Verify `start_startup()` automatically opts generated products into the
      pre-AL SII gate.

Validation:

```bash
cargo test -p esop-product-config
cargo test -p esop-cfggen
make cfggen-example
git diff --exit-code -- config/examples/sim-dual-axis/expected
```

## 4. Cross-layer verification and docs

- [x] Run scheduler/Linux simulation tests to prove the new Startup action uses
      existing single-request ownership.
- [x] Update product contract, requirements/PRD, README, capability manifest,
      and Trellis backend spec.
- [x] Run formatting, focused checks, and full repository CI.

Validation:

```bash
cargo test -p esop-ethercat-linux-port scheduled_domains
make capability-manifest
make ci
```

## Review Gates

- No AL action appears before a matching live signature for opted-in profiles.
- No partial candidate or verified signature survives a failure or restart.
- Existing legacy Startup tests remain unchanged in behavior.
- Core/product crates still pass the `aarch64-unknown-none` no-std check.
- Documentation does not claim response authenticity, HIL, WCET, FMMU/DC
  discovery, or safety certification.

## Risk And Rollback Points

- `startup.rs` action routing is the highest-risk ownership boundary; keep each
  action variant distinct and add pool/stale-response tests before broader work.
- Generated artifact shape changes must be committed with generator and runtime
  constructors in one batch.
- If fixed Startup memory becomes unacceptable in target qualification, retain
  the signature/profile contract and move scratch ownership to a caller-bound
  verified Startup wrapper in a follow-up; do not weaken fail-closed behavior.
