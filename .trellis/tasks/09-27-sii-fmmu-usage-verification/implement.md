# Implementation Plan

1. Add the core FMMU usage enum/profile/category parser and exhaustive unit
   tests in `sii.rs`; export the types from `lib.rs`.
2. Extend `SiiConfigurationCandidate`, progress/error handling, atomic category
   application, and schema-v2 signature hashing/tests.
3. Extend ESI parsing and generator models/templates for repeated `<Fmmu>`
   declarations; add invalid-input and propagation tests.
4. Extend `ProductSlaveConfig` and startup signature construction with the
   generated profile; validate deterministic PDO-group-to-FMMU compatibility.
5. Add Startup count/evidence/AL-order tests and update simulated SII fixtures to
   carry category `0x0028` where the generated expectation requires it.
6. Regenerate committed config artifacts and run focused crate tests.
7. Update PRD/requirements/status/capability docs, run Trellis quality checks and
   the full local CI command set.
8. Commit and push the implementation, wait for exact-SHA GitHub Actions, record
   acceptance evidence, archive the task, update the journal, then push the
   final documentation commit and verify its exact SHA.

## Validation Commands

```bash
cargo test -p esop-ethercat-core sii
cargo test -p esop-ethercat-core startup
cargo test -p esop-cfggen
cargo test -p esop-product-config
cargo test -p esop-ethercat-linux-port --test scheduled_domains
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
make ci
```

`make ci` is the repository policy gate: it includes formatting/diff checks,
workspace checks/tests/Clippy, release and `aarch64-unknown-none` builds,
capability/protobuf/config/build/performance/qualification validators, BPF C
syntax, and Zenoh feature compilation. GitHub Actions additionally runs the
full CO-RE BPF build, live Zenoh router tests, and privileged eBPF runtime jobs.

## Risk And Rollback Points

- Signature schema change: update all generated fixtures and tests in one
  atomic commit; never accept mixed v1/v2 evidence.
- ESI naming variance: accept only documented aliases and fail on unknown text
  instead of guessing.
- FMMU direction terminology: SII `Outputs` corresponds to EtherCAT RxPDO
  (master-to-slave), while ESC register `fmmu_type` uses `2`; keep the two enums
  separate and cover the mapping with tests.
- Startup atomicity: assign verified evidence only after FMMU, signature, and DC
  checks all succeed.
