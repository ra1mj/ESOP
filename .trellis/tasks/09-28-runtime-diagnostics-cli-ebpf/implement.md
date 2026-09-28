# Implementation plan

1. [x] Add the append-only Protobuf operational, Domain, and eBPF observation
   messages and extend schema compatibility/round-trip tests.
2. [x] Extend `ProcBufProjector` to build operational status from the same accepted
   snapshot and add exact cross-layer projection tests.
3. [x] Normalize zero query boot IDs to the current provider boot and add strict
   mismatch regressions.
4. [x] Add a reusable typed Zenoh query client operation with bounded request and
   reply validation.
5. [x] Add `esop-cli` library and `esop` binary with the initial command registry,
   typed renderers, doctor exit policy and bounded incremental watch.
6. [x] Add parser/renderer tests and a live local-router query test covering one
   projected state and one eBPF incident.
7. [x] Update README, capability manifest, software PRD and backend quality guidance
   with command ownership, read-only behavior, and qualification limits.
8. [x] Run focused tests, `make ci`, `git diff --check`, review the complete diff,
   commit, push, archive the child task and record the session.

## Completion evidence

- `cargo test -p esop-cli --all-features`
- `make test-zenoh` (two gateway tests and one CLI query/render test)
- `make ci`
- `git diff --check`

## Risk and rollback points

- Keep Protobuf changes append-only; never modify the frozen baseline fixture.
- Project operational values in `ProcBufProjector`, not in command renderers.
- Treat absent operational/eBPF evidence as unavailable or degraded, never
  healthy by default.
- Keep zero boot normalization inside typed read-only query decoding; command
  ingress continues to require exact identity.
- Bound watch intervals, limits and iterations before opening the transport.
- Do not add CLI/Aya/Zenoh dependencies to no_std or cycle crates.

## Validation commands

```text
cargo test -p esop-proto
cargo test -p esop-ipc --all-features
cargo test -p esop-zenoh-gateway --all-features
cargo test -p esop-cli --all-features
make proto-schema
make ci
git diff --check
```
