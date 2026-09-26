# Implementation Plan

## Ordered Work

- [x] Add copyable Startup configuration-service requirements, validation, and
      the optional PREOP barrier without changing default startup behavior.
- [x] Preserve the verified slave table across the barrier and resume legal AL
      transitions to SAFEOP/OP without rescanning or rereading SII.
- [x] Extend scheduler preselection to validate required bindings and release
      Startup only from actual Complete PDO/Mapping/DC controller phases.
- [x] Keep faulted configuration services selected and add typed missing/
      incomplete service behavior without caller readiness overrides.
- [x] Update lifecycle exhaustive matches and verify Topology remains false at
      the barrier and true only after final Startup Ready.
- [x] Add core and Linux simulated-port tests for one/multiple slaves,
      all-service completion, missing service, fault, AL error, restart, and
      no-rescan SAFEOP/OP progression.
- [x] Update product/PRD/EtherCAT/capability/README/Trellis contracts while
      retaining batch-generation, full WKC, physical HIL, and WCET limits.

## Focused Validation

```text
cargo test -p esop-ethercat-core startup --all-features
cargo test -p esop-ethercat-core production_service --all-features
cargo test -p esop-ethercat-linux-port --test scheduled_domains --all-features
cargo test -p esop-lifecycle-guard --all-features
cargo clippy -p esop-ethercat-core -p esop-ethercat-linux-port -p esop-lifecycle-guard --all-targets --all-features -- -D warnings
cargo check -p esop-ethercat-core --target aarch64-unknown-none
```

## Final Gates

```text
make ci
make bpf CLANG="$HOME/.local/opt/clang14/usr/bin/clang-14"
make test-zenoh
```

## Review Gates

- Confirm no barrier release is derived from a caller boolean.
- Confirm all expected slaves reach PREOP before configuration begins.
- Confirm release checks the frozen required-service set and actual controller
  phases, with missing bindings failing before AL resumes.
- Confirm Startup records and identities survive the barrier unchanged.
- Confirm OP always follows an observed SAFEOP step.
- Confirm existing no-barrier callers and request ownership remain unchanged.
- Confirm all loops, storage, errors, and reports stay fixed-capacity/no_std.

## Rollback Point

Keep the barrier and scheduler release in one feature commit. If release cannot
be proven from actual controller state without weakening service ownership,
revert the complete change rather than exposing a manual readiness override.
