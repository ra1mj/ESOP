# Quality Guidelines

The canonical local quality gate is `make ci`. It runs formatting and diff
checks, workspace type checking, all-feature tests, Clippy with warnings
denied, release compilation, the core `aarch64-unknown-none` check, and the
BPF C syntax check. `make bpf` is the additional CO-RE object build and needs
clang, bpftool, kernel BTF, and a Linux BPF-capable host.

## Forbidden Patterns

- Do not add heap allocation, blocking locks, unbounded loops, sleeping, or
  text logging to the activated EtherCAT cycle path.
- Do not use `volatile` as a substitute for synchronization or DMA ownership.
- Do not expose Aya or other Linux-only dependencies from the `no_std` core.
- Do not bypass generation, address, size, type, or working-counter checks when
  consuming a received datagram.
- Do not release a DMA-owned descriptor before the platform reports completion.

## Required Patterns

- Use fixed-capacity arrays, caller-owned buffers, and explicit error enums in
  real-time code. `DmaDescriptorRing` and `RxIndexTable` are the reference
  implementations.
- Make ownership transitions and cache maintenance visible at DMA boundaries.
- Keep frame plans immutable during the active cycle and bind RX expectations
  to the same frame or descriptor generation.
- Add a regression test for every lifecycle or validation change. Prefer
  public integration tests when a change crosses crates.
- Preserve the existing `EthercatPort` copy-compatible path while adding a
  separate DMA trait for zero-copy platform adapters.
- Keep ProcBuf as a standalone `no_std` ABI layer. Use its header/layout hash
  to reject robot/boot/capacity mismatches, and use the double-page ownership
  protocol rather than an unchecked sequence-only copy.
- CiA 402 mode confirmation is not sufficient for motion: cyclic output must
  also require `OperationEnabled`, MLG permission, and a seeded/limited first
  setpoint.
- Keep the CiA 402 EtherCAT adapter behind the profile `ethercat` feature and
  reuse `esop_ethercat_core::PdoEntry` for bit access. Validate standard object
  identity, direction, width, signedness, and overlap before activation; keep
  control/mode handshaking separate from cyclic target writes.
- A typed CiA 402 cyclic write must require all four motion gates
  (lifecycle permit, confirmed mode, Operation Enabled, and valid setpoint) and
  an Operation Enabled Controlword. Preflight every destination field before
  mutating the caller-owned image so rejected writes are atomic.
- Device lifecycle transitions must remain explicit and bounded. A faulted
  device cannot jump directly to `Active` or `Cyclic`; recovery must create a
  new generation and pass configuration phases again.
- Domain/PDO registration must finish before activation. Keep Domain-local PDO
  offsets and datagram payload offsets deterministic, reject process-image and
  logical-address overlap plus global datagram-index reuse, and lock the
  registry after the schedule is activated.
- SII-derived Domain registration must validate the candidate's FMMU/SyncManager
  mapping and Rx/Tx-to-unified-image translation before publishing any
  Domain/PDO entries. Apply the registry update transactionally so insufficient
  image/capacity or a logical-base mismatch cannot leave a partial
  configuration.
- Build multi-frame plans in fixed-capacity temporary storage and publish them
  only after every Domain datagram and the schedule validate. Split only on
  per-frame capacity or encoded MTU; reject a single datagram that cannot fit.
- Auto-bind SII segments only when their logical and physical offsets and bit
  length are byte-aligned. Bit-packed segments require an explicit aggregate
  datagram so byte-oriented Domain staging cannot overwrite neighboring bits.

## Testing Requirements

Run `make ci` before handing off a change. Run `make test-hil` when modifying
the Linux port or simulator. New DMA behavior must cover ownership, cache
ordering, stale handles, invalid lengths, error rollback, and at least one
end-to-end simulator path. New eBPF ABI fields must have fixed-size decode and
invalid-discriminant tests. New CiA 402 PDO fields must have public API tests
for all supported modes and a failure-path test proving the output image is
unchanged.

## Code Review Checklist

- Is the worst-case loop bounded by a static capacity or explicit budget?
- Are errors propagated without silently publishing partial process data?
- Are descriptor generation and cache/barrier ordering correct?
- Does the test exercise the public API and the failure path?
- Were `make ci` and the relevant cross-target checks run?
