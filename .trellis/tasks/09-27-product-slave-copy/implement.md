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
