# Implementation Plan

1. Add the fixed-capacity register request controller, request/action/status
   types, validation, FIFO progression, pool enqueue/completion and unit tests.
2. Export the API from `esop-ethercat-core` and integrate the optional service
   into all exhaustive production scheduler dispatch points.
3. Add scheduler priority and Linux simulated cycle tests, including a
   multi-cycle read and process/DC/control TX ordering assertion.
4. Update backend quality guidance with the asynchronous register service
   ownership rule.
5. Update `REG-001` requirements text and add an evidence-bound capability
   manifest entry.
6. Run focused formatting/tests, `make test-hil`, then `make ci`.
7. Review the diff, archive the Trellis task, record the developer journal,
   commit, push `main`, and wait for GitHub Actions on the exact pushed SHA.

## Risk Points

- `production_service.rs` exhaustively dispatches service kinds in several
  methods; every match and priority list must include the new service.
- Pool handle cleanup must occur on every terminal/mismatch path.
- Generic defaults must preserve all existing `ScheduledProductionServices`
  call sites.
- Tests must distinguish software ordering evidence from target timing claims.

## Validation

```text
cargo fmt --all -- --check
cargo test -p esop-ethercat-core --all-features
cargo test -p esop-ethercat-linux-port --test scheduled_domains
make test-hil
make ci
```
