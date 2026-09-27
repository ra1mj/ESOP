# Implementation Plan: EtherCAT Recovery Integration and Qualification

## 1. Unified Public Surface

- [x] Add `recovery.rs` with unified kind, coarse phase, fault, status, and
      result types.
- [x] Implement additive `From` conversions and common `const` metadata
      accessors without hiding operation-specific detail.
- [x] Export the new public API from `esop-ethercat-core`.

Validation:

```bash
cargo test -p esop-ethercat-core recovery
cargo check -p esop-ethercat-core --no-default-features
```

Rollback point: revert the new module and export only; existing controllers are
unchanged.

## 2. Fixed-Capacity Diagnostics

- [x] Add submitted/progress/completed/faulted diagnostic event codes.
- [x] Add a fixed SPSC queue with per-kind deduplication and an observable lost
      event count.
- [x] Cover unchanged observation, sequence replacement, progress, terminal
      success, terminal fault, exact typed-fault retention, and overflow.

Validation:

```bash
cargo test -p esop-ethercat-core recovery
```

Rollback point: remove the diagnostic queue without changing the unified
snapshot enums.

## 3. Mixed Cyclic-Load Qualification

- [x] Extend scheduler tests for the full recovery priority order.
- [x] Extend Linux simulations with unified snapshots/diagnostics and explicit
      assertions for LRW/FRMW ordering, one control slot, no in-flight
      retransmission, post-receive deadline evidence, and no lifecycle bypass.
- [x] Reuse existing virtual-port and ready-Startup fixtures; do not create a
      second protocol implementation in tests.

Validation:

```bash
cargo test -p esop-ethercat-core production_service
cargo test -p esop-ethercat-linux-port --test scheduled_domains explicit_
make test-hil
```

## 4. Documentation and Capability Boundary

- [x] Update `REC-001` and ETG/CiA 402 requirement commentary.
- [x] Update the software PRD R2 release-boundary text.
- [x] Update `ethercat_explicit_recovery` capability evidence and limitations.
- [x] Add `.trellis/spec/backend/ethercat-recovery-integration.md` and link it
      from the backend index and relevant quality guidance.
- [x] Keep physical timing, HIL, interoperability, ETG, STO, and functional
      safety gaps explicit.

Validation:

```bash
python3 scripts/validate-capability-manifest.py
git diff --check
```

## 5. Full Qualification and Delivery

- [x] Run focused formatting, Clippy, unit, Linux simulation, and `no_std`
      checks.
- [x] Run `make test-hil` and `make ci`.
- [ ] Run Trellis check/spec-update/finish flow.
- [ ] Commit, push to `ra1mj/ESOP`, and wait for exact GitHub Actions success.

Final validation:

```bash
cargo fmt --all -- --check
cargo clippy -p esop-ethercat-core -p esop-ethercat-linux-port --all-targets -- -D warnings
cargo check -p esop-ethercat-core --no-default-features
make test-hil
make ci
```
