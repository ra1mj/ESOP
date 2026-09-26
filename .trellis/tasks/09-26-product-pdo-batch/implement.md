# Implementation Plan

## Ordered Work

- [x] Add fixed-capacity PDO job/batch plan types and transactional validation.
- [x] Add the batch controller with bounded empty-job skipping, per-job
      generation, fault retention, explicit restart, and status reporting.
- [x] Generalize `ScheduledPdoConfiguration` over single/batch modes and gate
      Startup release on whole-batch completion.
- [x] Add product mailbox bindings and exact all-slave batch construction.
- [x] Add core, scheduler, generated-product, and Linux simulated integration
      tests for ordering, readback, faults, empty jobs, restart, and release.
- [x] Synchronize product/PRD/EtherCAT/capability/README/Trellis contracts.

## Focused Validation

```text
cargo test -p esop-ethercat-core pdo_config --all-features
cargo test -p esop-ethercat-core production_service --all-features
cargo test -p esop-product-config --all-features
cargo test -p esop-ethercat-linux-port --test scheduled_domains --all-features
cargo clippy -p esop-ethercat-core -p esop-product-config -p esop-ethercat-linux-port --all-targets --all-features -- -D warnings
cargo check -p esop-ethercat-core --target aarch64-unknown-none
cargo check -p esop-product-config --target aarch64-unknown-none
```

## Final Gates

```text
make ci
make bpf CLANG="$HOME/.local/opt/clang14/usr/bin/clang-14"
make test-zenoh
```

## Review Gates

- Confirm the batch cannot publish Complete while a later job exists.
- Confirm scheduler release uses batch state, never current-controller Complete.
- Confirm only the existing controller consumes PDO/mailbox responses.
- Confirm faults retain exact job/index and require explicit restart.
- Confirm product bindings cover each slave exactly once before publication.
- Confirm loops and storage remain statically bounded and allocation-free.
- Confirm the single-plan constructor and behavior remain unchanged.

## Rollback Point

Keep core batch types, scheduler integration, product builder, tests and claims
in one feature commit. Revert the complete change if whole-batch completion
cannot be proven without caller readiness booleans or duplicate transport code.
