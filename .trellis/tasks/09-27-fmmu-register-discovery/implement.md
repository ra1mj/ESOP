# Implementation Plan

1. Add `fmmu_discovery.rs` with the descriptor, fixed bank, action/progress/error
   types, bounded controller, exact control-pool integration, and unit tests.
2. Export the new contracts and single-source FMMU capacity/register constants
   from the core crate.
3. Add Startup phase/action/error/evidence state, per-profile sequencing,
   atomic publication, restart clearing, mapping-start bridge, and focused tests.
4. Extend `MappingConfigController` with verified-bank preflight and complete
   bank clear/readback phases while preserving the legacy `start` API.
5. Exercise the enhanced mapping path through the existing production service
   scheduler and deterministic Linux simulator.
6. Update README, software PRD, capability manifest, and backend product spec
   with exact implemented and remaining boundaries.
7. Run focused formatting/tests, full core and Linux scheduled-domain tests,
   then `make ci` and any environment-supported BPF/Zenoh gates.
8. Complete acceptance evidence, update the spec if implementation discoveries
   change an executable contract, commit, push `main`, and verify the exact
   pushed SHA's GitHub Actions `quality` run.

## Risk and Rollback Points

- `startup.rs` has a broad test surface; preserve legacy profile sequencing and
  update only expected-SII helpers.
- Mapping phase additions must not alter action ownership or request release.
- Do not publish staged descriptor banks before SII/DC verification succeeds.
- Keep all loops bounded by the single exported maximum of 16 FMMU pages.

## Validation Commands

```bash
cargo fmt --all -- --check
cargo test -p esop-ethercat-core fmmu --no-fail-fast
cargo test -p esop-ethercat-core startup --no-fail-fast
cargo test -p esop-ethercat-core mapping_config --no-fail-fast
cargo test -p esop-ethercat-core --no-fail-fast
cargo test -p esop-ethercat-linux-port --test scheduled_domains
make ci
```
