# Implementation Plan

1. Add `watchdog.rs` to `esop-ethercat-core` with raw register constants,
   strict config/plan contracts, the fixed-capacity write/readback controller,
   control-pool integration, complete-only evidence, and focused tests.
2. Export the watchdog API and add the opt-in Startup configuration-service
   bit without changing existing masks or legacy behavior.
3. Integrate the controller into every production scheduler path with fixed
   priority after PDO Configuration and before Mapping.
4. Extend `esop-product-config` with the optional per-slave field, typed plan
   errors, deterministic `watchdog_plan()`, and focused runtime tests.
5. Extend cfggen's strict schema, validation, generated semantic model,
   normalized/inventory JSON, C header/data, Rust module, and rejection tests.
6. Configure both simulator drives, regenerate all six golden artifacts, and
   update exact hash assertions and generated-product tests.
7. Add public Linux scheduler integration proving cross-cycle watchdog
   write/readback, priority, fault handling, and PREOP gating.
8. Update README, product/master requirements, capability manifest, product
   configuration docs, and backend executable spec with software-only limits.
9. Run focused tests, all affected crate suites, Linux scheduled-domain tests,
   then `make ci` and environment-supported BPF/Zenoh gates.
10. Record acceptance evidence, update the Trellis task/spec/journal, commit,
    push `main`, and verify the exact SHA's GitHub Actions `quality` run.

## Risk and Rollback Points

- `production_service.rs` routes one shared request through many variants;
  every match must be exhaustive and request ownership must remain exact.
- `ProductSlaveConfig` is emitted and instantiated in several fixtures; update
  all constructors without weakening strict runtime validation.
- C and Rust generated structures must express field presence separately from
  raw zero so default preservation cannot be confused with a write.
- Golden hash changes must arise only from intentional simulator watchdog
  declarations; determinism checks must still compare byte-for-byte output.
- Keep all controller loops bounded by `MAX_SLAVES`; do not allocate or block.

## Validation Commands

```bash
cargo fmt --all -- --check
cargo test -p esop-ethercat-core watchdog --no-fail-fast
cargo test -p esop-ethercat-core production_service --no-fail-fast
cargo test -p esop-product-config --no-fail-fast
cargo test -p esop-cfggen --no-fail-fast
cargo test -p esop-ethercat-linux-port --test scheduled_domains
make ci
```
