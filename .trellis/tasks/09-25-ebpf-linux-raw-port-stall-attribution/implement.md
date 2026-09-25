# Implementation Plan: eBPF Linux raw port stall attribution

## Ordered Work

1. Add bounded raw-port operation/outcome types, exact marker symbols, syscall
   guards, and exact-symbol/pure encoding tests in
   `esop-ethercat-linux-port`.
2. Extend agent evidence/incident enums, detail documentation, classification,
   and positive/negative correlation tests.
3. Extend C context/stats ABIs, add the bounded per-thread operation map, add
   raw-port begin/end uprobe programs, and emit fixed raw-port stall evidence.
4. Extend the Aya runtime configuration, context updates, statistics,
   discriminant decode, attach masks/specs, generalized transactional pair
   attachment, and ABI/config/decode/aggregation tests.
5. Update runtime-observability documentation, capability evidence/limits,
   README status if applicable, and the backend quality contract.
6. Run focused tests, format/lint/full CI, BPF syntax validation, and a real
   CO-RE object build with the repository-qualified Clang 14 toolchain.
7. Review the final diff for scope/claim accuracy, commit, push to
   `ra1mj/ESOP`, monitor GitHub Actions, archive the task, and record the
   Trellis journal commit.

## Validation Commands

```bash
cargo test -p esop-ethercat-linux-port
cargo test -p esop-ebpf-agent
cargo test -p esop-ebpf-runtime
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
make test
make ci
make bpf-syntax
env LD_LIBRARY_PATH=/home/mj/.local/opt/clang-14/usr/lib/x86_64-linux-gnu \
  make -C bpf clean all CLANG=/home/mj/.local/opt/clang-14/usr/bin/clang-14
```

## Review Gates

- Confirm no marker is added to the simulator or no-std EtherCAT core.
- Confirm `errno` is captured before the terminal marker on syscall failure.
- Confirm every matched BPF end deletes state before later validation.
- Confirm raw-port and gateway maps, thresholds, epochs, counters, kinds, and
  attach bits are independent.
- Confirm all loops/maps remain statically bounded and the port adds no lock,
  allocation, logging, or clock read.
- Confirm context/stats sizes match across C/Rust and evidence remains 96 bytes.
- Confirm docs state syscall-boundary evidence, not complete-cycle causality or
  production realtime qualification.

## Risk And Rollback Points

- Marker ABI errors are caught by an integration binary that exact-links the
  symbols and by fixed operation/outcome tests.
- BPF verifier/layout regressions are caught by static assertions, Rust ABI
  tests, syntax validation, and real object compilation.
- Partial uprobe capability is prevented by pair rollback; optional mode must
  leave previous capabilities untouched.
- If the BPF design cannot pass the real build without widening scope, revert
  the raw-port observation increment as a unit rather than weakening event
  validation or reusing misleading gateway fields.
