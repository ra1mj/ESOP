# Implementation Plan

## Ordered Work

- [x] Add the action-checked PDO mailbox transport error and fault entry point,
      with focused no-advance/no-partial-state tests.
- [x] Add `ScheduledPdoConfiguration` and a defaulted PDO operation capacity to
      `ScheduledProductionServices` while preserving the existing constructor.
- [x] Add PDO Configuration service/progress/fault variants and scheduler-held
      immutable PDO action identity.
- [x] Reuse the mailbox enqueue and `run_dc_and_mailbox_cycle` path to prepare,
      submit, wait, retry, complete, and fault one PDO SDO transaction.
- [x] Extend selection, request matching, readiness, controller fault, recovery,
      and explicit restart behavior for PDO Configuration.
- [x] Map PDO Configuration to the lifecycle Configuration/CoE gate and update
      every exhaustive service-kind match.
- [x] Add Linux simulated-port integration for exact download/upload readback,
      fixed priority, cross-generation ownership, Prepared rebuild, retry,
      timeout/mismatch, fault blocking, restart, and report confirmation.
- [x] Update product, lifecycle, robotics, PRD, capability, README, and Trellis
      contracts without claiming PREOP orchestration or physical HIL.

## Validation

Run focused checks during implementation:

```text
cargo test -p esop-ethercat-core pdo_config --all-features
cargo test -p esop-ethercat-core production_service --all-features
cargo test -p esop-ethercat-linux-port --test scheduled_domains --all-features
cargo test -p esop-lifecycle-guard --all-features
cargo test -p esop-product-config --all-features
cargo clippy -p esop-ethercat-core -p esop-ethercat-linux-port -p esop-lifecycle-guard --all-targets --all-features -- -D warnings
cargo check -p esop-ethercat-core --target aarch64-unknown-none
```

Final gates:

```text
make ci
make bpf CLANG="$HOME/.local/opt/clang14/usr/bin/clang-14"
make test-zenoh
```

## Review Gates

- Confirm no raw CoE payload is inserted directly into `ControlRequestPool`.
- Confirm one and only one owner releases each request handle.
- Confirm poll/retry delays leave the pool slot free.
- Confirm cross-generation completion is limited to the unchanged in-flight
  request and does not relax Domain/DC generation validation.
- Confirm PDO operation index advances only after mailbox completion and exact
  upload readback.
- Confirm lifecycle readiness cannot be supplied or restored by the caller.
- Confirm all loops and buffers remain statically bounded and `no_std`.

## Rollback Point

Keep the scheduler/mailbox composition in one feature commit. If the bridge
cannot preserve existing request ownership or API compatibility, revert the
complete commit rather than leaving a partially scheduled PDO path.
