# Implementation Plan

1. Add core register constants and the allocation-free Requesting ID controller
   with exhaustive unit tests.
2. Export the new API and integrate its phase/action/progress/error/evidence into
   Startup before mailbox/SII/AL branching.
3. Extend ESI parsing and strict tests for `Info/IdentificationReg134`.
4. Extend product manifest/model, normalized artifacts, semantic hashing,
   C/Rust generation, inventory/build input, and generator rejection tests.
5. Extend `esop-product-config` static fields, enabled-implies-supported
   validation, Startup profile attachment, and anti-tamper tests.
6. Add distinct Requesting IDs to the simulated drives, regenerate checked-in
   artifacts, and update expected hashes.
7. Add core and Linux production-scheduler integration tests for ordering,
   swapped/mismatched values, timeout, and lifecycle Topology fail-closed.
8. Update product/master/PRD docs, capability manifest, Trellis specs, and known
   limitations with evidence-bound wording.
9. Run focused crate tests, generator determinism checks, `make test-hil`, and
   full `make ci`.
10. Commit implementation, archive the Trellis task, record the journal, push
    `main`, and wait for the exact GitHub Actions SHA to pass.

## Risk Points

- AL Status Code is reused for ordinary AL errors; Requesting ID may only run in
  the explicit pre-AL phase and must require ID Loaded bit 5 before reading it.
- Generic Startup action helpers must preserve exact payload/response length so
  the control pool cannot confuse the write, poll, and value-read transactions.
- Generated support and expectation fields must affect semantic/config hashes;
  otherwise a stale artifact could silently skip the gate.
- Simulator helpers that currently special-case Startup phases must be updated
  without weakening action identity checks.

## Validation

```text
cargo test -p esop-ethercat-core --all-features
cargo test -p esop-cfggen
cargo test -p esop-product-config
cargo test -p esop-ethercat-linux-port --test scheduled_domains
make test-hil
make ci
```
