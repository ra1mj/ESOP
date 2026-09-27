# Implementation Plan

1. Extend core slave-copy types with a fixed-capacity immutable plan set,
   overlap validation, and read-only plan metadata. Add focused unit tests.
2. Extend product runtime static/activated contracts, retain PDO handles during
   activation, rebuild the plan set transactionally, and add activation tests.
3. Extend cfggen schema and normalization, resolve semantic PDO references,
   validate through the active core registry, update metrics, and render JSON,
   C, and Rust records.
4. Add the simulator ESI fields and manifest copy declaration, regenerate all
   checked-in artifacts, and update exact generated-product assertions.
5. Add generator negative tests for unknown references, wrong direction,
   same-slave/width violations, overlap, and unknown fields.
6. Add a public generated-product execution test covering valid copy, invalid
   WKC fallback, stale fallback, and target frame payload.
7. Update product configuration and EtherCAT requirement documentation with
   the generated ownership contract and evidence boundary.
8. Run focused crate tests, regenerate/diff-check artifacts, then run
   `make ci`. Review the full diff before commit and push.

## Rollback points

- Core plan-set changes are independently testable before schema changes.
- Generated artifact changes are published only after cfggen succeeds.
- Runtime activation returns no partial `ActivatedProduct` or plan set.

## Validation

```text
cargo test -p esop-ethercat-core --test slave_copy
cargo test -p esop-cfggen --test generation
cargo test -p esop-product-config --test generated_product
make cfggen-example
git diff --exit-code -- config/examples/sim-dual-axis/expected
make ci
```

## Implementation Result

Accepted on 2026-09-27 in implementation commit
`935a009281bdcb7201d9c99fa2e58b8fe71a209d`.

- Added strict semantic `slave_copies` declarations and deterministic JSON,
  C, Rust, hash, and build-report evidence.
- Added fixed-capacity immutable plan ownership with transactional capacity and
  target-overlap checks in the `no_std` core and runtime product activation.
- Runtime activation retains only referenced source, target, and quality PDO
  handles, preserving `PDOS` as a per-Domain capacity before rebuilding every
  plan against the activated registry.
- Added the simulator drive-position-to-IO mirror and public cross-Domain tests
  for valid copy, target frame bytes, stale fallback, invalid-WKC fallback,
  tampered indices, capacity, reference, direction, width, and overlap faults.
- Regenerated all six checked-in artifacts with configuration hash
  `e241ac19602f9adc68d206e726ed048fa3f915e34b04e6bd35a45af9774b6241`.
- Passed focused crate suites, byte-for-byte generated-artifact comparison,
  local `make ci`, and GitHub Actions `quality` run `36300420567` for the exact
  implementation SHA.

Acceptance is limited to generated and simulated software evidence. Physical
slave mapping/readback, timing/WCET qualification, HIL, and functional-safety
qualification remain out of scope. Automatic publication into a long-lived
stable process image remains a later cycle-owner integration task.
