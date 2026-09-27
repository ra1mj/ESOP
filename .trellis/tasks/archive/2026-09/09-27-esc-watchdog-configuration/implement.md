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

## Implementation Result

Accepted on 2026-09-27 in implementation commit
`e7c8e8e7f1189ce273748afcc843f720926df713`.

- Added strict product-owned ESC watchdog divider and process-data interval
  configuration with deterministic normalized, inventory, C, Rust, and hash
  artifacts.
- Added a fixed-capacity write/readback controller for `0x0400/2` and
  `0x0420/2` with exact request ownership, WKC, length, deadline, readback,
  first-failure, and complete-only evidence contracts.
- Integrated the controller between PDO configuration and mapping, including
  cross-cycle request retention and the opt-in PREOP startup barrier.
- Added generated-product, scheduler, Linux simulation, lifecycle, rejection,
  compatibility, and fault-path tests; the simulator exercises both registers
  on two drives while preserving the IO slave default.
- Passed every focused crate suite, `make cfggen-runtime-example`, capability
  validation, and the complete local `make ci` gate.
- GitHub Actions `quality` run `36298255226` passed for the exact implementation
  SHA, including Rust, Zenoh, BPF build, and privileged eBPF runtime jobs.

Acceptance is limited to deterministic software-provided responses. Physical
watchdog expiration, response authenticity, target timing/WCET, HIL,
interoperability, ETG conformance, and functional-safety qualification remain
out of scope.
