# Implementation Plan

1. Extend `SlaveCopyPlan` with product-image target metadata and add the
   double-page `SlaveCopyProcessImage` plus fixed publication report types.
2. Extend the sealed scheduled Domain interface with committed input access;
   implement transactional, cycle-ordered copy publication in
   `ScheduledDomainBank` with focused unit tests.
3. Add shared-process-image entries to `ScheduledAuxiliaryOutputs`, preserve
   the static constructor, and thread the published image through bank-aware
   lifecycle output paths.
4. Add the copy-enabled production-owner constructor, completion variants,
   fault phase, and release evidence. Cover replay, motion-target rejection,
   publication failure, and exact due-copy counts.
5. Add a generated-product cross-layer Linux simulation proving fresh,
   invalid-WKC, stale, and non-due behavior reaches exact next-generation
   frame bytes without manual plan execution.
6. Update product configuration, quality, README, and software PRD contracts
   with the automatic publication path and evidence limits.
7. Run focused tests and Clippy, `aarch64-unknown-none` checks, generated
   artifact diff checks, then full `make ci`. Review, commit, push, and verify
   the exact GitHub Actions run.

## Validation

```text
cargo test -p esop-ethercat-core --test slave_copy
cargo test -p esop-ethercat-core scheduled_domains
cargo test -p esop-lifecycle-guard --all-features
cargo test -p esop-ethercat-linux-port --test verified_stop_cycle
cargo test -p esop-product-config --all-features
cargo clippy -p esop-ethercat-core -p esop-lifecycle-guard -p esop-product-config -p esop-ethercat-linux-port --all-targets --all-features -- -D warnings
cargo check -p esop-ethercat-core -p esop-product-config --target aarch64-unknown-none
make cfggen-runtime-example
make ci
```

## Rollback Points

- Core image publication is independently testable before lifecycle changes.
- Static auxiliary-output construction remains available throughout.
- Copy-enabled owner APIs are additive; if integration fails, they can be
  removed without changing generated artifact schemas.

## Review Gates

- Confirm every plan is preflighted before staging mutation.
- Confirm a copy-enabled owner has no no-copy receive-completion escape hatch.
- Confirm shared-image bounds are checked before frame acquisition or TX.
- Confirm motion-Domain targets and receive replay fail closed.
- Confirm documentation does not claim HIL, WCET, or functional safety.
