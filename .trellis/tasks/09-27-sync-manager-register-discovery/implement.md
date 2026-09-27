# Implementation Plan

1. Add `sync_manager_discovery.rs` with the descriptor, fixed bank,
   action/progress/error types, bounded controller, exact control-pool
   integration, and focused unit tests.
2. Export the new contracts and keep SyncManager register constants/capacity
   single-sourced from the core crate.
3. Add Startup phase/action/error/evidence state, FMMU-then-SM sequencing,
   atomic multi-evidence publication, restart clearing, and focused tests.
4. Extend `MappingConfigController` with a full verified-register preflight and
   complete SyncManager clear/readback phases while preserving legacy and
   FMMU-only APIs.
5. Exercise the full enhanced mapping path through the existing production
   service scheduler and deterministic Linux simulator.
6. Update README, software/master requirements, capability manifest, and
   backend product specification with exact implemented/remaining boundaries.
7. Run focused formatting/tests, full core and Linux scheduled-domain tests,
   then `make ci` and environment-supported BPF/Zenoh gates.
8. Complete acceptance evidence, update executable specs, commit, push `main`,
   and verify the exact pushed SHA's GitHub Actions `quality` run.

## Risk and Rollback Points

- `startup.rs` has a broad test surface; preserve legacy profile sequencing and
  only add traffic for expected-SII profiles.
- Mapping phase additions must preserve FMMU-only action order and control-pool
  ownership.
- Do not publish either staged register bank before SII/DC verification.
- Keep all loops bounded by the shared maximum of 16 SyncManager pages.

## Validation Commands

```bash
cargo fmt --all -- --check
cargo test -p esop-ethercat-core sync_manager_discovery --no-fail-fast
cargo test -p esop-ethercat-core startup --no-fail-fast
cargo test -p esop-ethercat-core mapping_config --no-fail-fast
cargo test -p esop-ethercat-core --no-fail-fast
cargo test -p esop-ethercat-linux-port --test scheduled_domains
make ci
```
