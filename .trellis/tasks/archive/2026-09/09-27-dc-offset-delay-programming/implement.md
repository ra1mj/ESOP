# Implementation Plan

## 1. Preserve DC range in immutable topology

- [x] Add `EscDcRange` to `DcTopologySlave` and populate it from scan evidence.
- [x] Update topology fixtures and equality assertions without changing
      physical-tree or propagation-delay behavior.

Validation:

```bash
cargo test -p esop-ethercat-core dc_topology
```

## 2. Implement the bounded clock controller

- [x] Add public config, phase, action, evidence, progress, and error types.
- [x] Validate and copy a topology-wide System-Time plan before first action.
- [x] Implement exact `0x0910/24` read and combined `0x0920/12` write sequence.
- [x] Implement 64-bit checked and 32-bit wrap-aware offset correction using
      elapsed monotonic time.
- [x] Enforce complete-only publication, diagnostic completed count, restart
      clearing, and fail-stop terminal handling.
- [x] Add focused arithmetic, sequencing, malformed response, timeout,
      topology-evidence, and partial-write tests.

Validation:

```bash
cargo test -p esop-ethercat-core dc_clock
```

## 3. Integrate scheduler, Startup barrier, and lifecycle gate

- [x] Add an optional scheduler clock-service attachment without changing the
      existing constructor.
- [x] Schedule clock programming before existing DC SYNC configuration and
      route actions/results through the shared control request owner.
- [x] Add a distinct Startup configuration-service requirement and fail-closed
      release checks.
- [x] Classify the service under the lifecycle configuration gate.
- [x] Add scheduler ordering/barrier/fault and lifecycle authorization tests.

Validation:

```bash
cargo test -p esop-ethercat-core production_service
cargo test -p esop-lifecycle-guard ethercat
```

## 4. Prove public request/RX behavior and sync documentation

- [x] Add one integration regression for 24-byte read and 12-byte write
      payloads with exact WKC and ownership.
- [x] Update README, master requirements, capability manifest, and backend
      product/DC specification.
- [x] Keep all application-time authenticity, runtime lock, physical timing,
      HIL, WCET, conformance, and functional-safety limitations explicit.

Validation:

```bash
cargo test -p esop-ethercat-core --test cycle
make capability-manifest
```

## 5. Run repository quality gates and publish

- [x] Run formatting, diff checks, focused/full tests, no-std, CI, and BPF.
- [x] Commit and push the implementation to `ra1mj/ESOP`.
- [x] Verify GitHub Actions against the exact pushed SHA; repair and repush on
      any failure.
- [x] Mark acceptance criteria, archive the task, update the developer journal,
      and push the completion metadata.

Validation:

```bash
cargo fmt --all -- --check
git diff --check
make no-std
make ci
make bpf
```

## Validation Results

Validated locally on 2026-09-27:

- `cargo test -p esop-ethercat-core --lib`
- `cargo test -p esop-ethercat-core --test cycle`
- `cargo test -p esop-lifecycle-guard --all-features`
- `cargo test -p esop-ethercat-linux-port --test scheduled_domains`
- `make capability-manifest`
- `make no-std`
- `make ci`
- `make bpf`
- GitHub Actions `quality` run `36279028999` passed for exact commit
  `559ca25697cdb3f64326479f9ecb2e4870838288`.
- GitHub Actions `quality` run `36279358839` passed for exact acceptance commit
  `096e46a0434fbf0136700153eeb991009eef40bb`.

## Review Gates

- No request is emitted before the full plan validates.
- Offset and delay are always written together as one 12-byte action.
- Exact WKC 1 and exact response length apply to every clock read/write.
- 32-bit wrap behavior is explicit and tested; 64-bit out-of-range deltas
  fault instead of truncating.
- Complete evidence is absent after any fault even when earlier hardware writes
  succeeded; diagnostic progress remains visible.
- Existing SYNC controller and scheduler constructor remain compatible.
- Generic control/RX code gains no EtherCAT register-specific behavior.
- Cyclic and configuration paths remain bounded, allocation-free, and
  non-blocking.

## Risk And Rollback Points

- Scheduler service enums are exhaustively matched across core and lifecycle;
  compile failures must be resolved without weakening default-deny gates.
- Existing topology fixtures may assume equality without the range field;
  update fixtures mechanically and preserve all prior assertions.
- Physical writes cannot be rolled back. A partial failure must remain a
  latched fault and must never publish a complete batch.
- The new controller and scheduler attachment can be reverted together without
  persistent data migration; retaining documentation claims without the
  service is not a valid partial rollback.
