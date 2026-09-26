# Implementation Plan

## Ordered Work

- [x] Add `PdoConfigStep`, include it in actions, and update controller state so
      each planned write alternates download and verification upload.
- [x] Add precise readback length/value errors and compare completed uploads
      before advancing `operation_index`.
- [x] Expand core PDO configuration tests for exact readback, segmented upload,
      mismatches, stale actions, generation, timeout, and operation ordering.
- [x] Add the shared maximum generated PDO-entry capacity and use it from
      cfggen instead of the private duplicate constant.
- [x] Add `ProductPdoStartupPlan`, product-level plan errors, and the bounded
      per-slave generated-plan builder.
- [x] Add product-config tests for exact drive/IO mapping and assignment
      sequences plus every fail-closed validation path.
- [x] Regenerate and compare the checked-in simulator artifacts; keep semantic
      hash and public generated field spelling stable.
- [x] Update product/robotics/PRD/capability/Trellis documentation with the
      delivered software evidence and remaining scheduler/HIL boundary.

## Validation

Run focused checks during implementation:

```text
cargo test -p esop-ethercat-core pdo_config --all-features
cargo test -p esop-product-config --all-features
cargo test -p esop-cfggen --all-features
make cfggen-example
make cfggen-runtime-example
```

Run the final workspace gates:

```text
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
make ci
make bpf CLANG="$HOME/.local/opt/clang14/usr/bin/clang-14"
make test-zenoh
```

Confirm the generated Rust module is byte-identical to the checked-in golden
and retain the core/product `aarch64-unknown-none` checks performed by
`make ci`.

## Review Gates

- Verify `operation_index` never advances on download acknowledgement alone.
- Verify mismatch errors are latched through the common fault path.
- Verify per-slave filtering prevents the same mapping index on another slave
  from entering the selected plan.
- Verify mapping and assignment ordering is deterministic under the checked-in
  generated order.
- Verify no generated schema/hash churn and no heap/`std` dependency enters
  runtime crates.
- Verify documentation does not claim production/HIL completion.

## Risk and Rollback Points

- The controller behavior intentionally doubles control transactions; focused
  tests must catch callers that assumed one action per write.
- Product plan scratch storage is startup-only but fixed at the cfggen hard
  limit; stack-size impact should remain visible in review.
- If action-step exposure causes downstream compilation failures, retain the
  internal step model and provide an accessor-compatible action enum before
  changing protocol behavior.
- If generated golden output changes unexpectedly, stop and identify semantic
  model drift rather than accepting regenerated files blindly.
