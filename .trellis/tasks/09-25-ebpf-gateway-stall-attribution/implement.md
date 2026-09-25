# Implementation plan: eBPF gateway stall attribution

## Ordered work

- [x] Add the Zenoh-feature marker ABI, non-zero request allocator, publish
  lifecycle guard, explicit outcomes, and exact-symbol integration test.
- [x] Extend the shared BPF/Rust runtime context and statistics layouts with
  gateway threshold, policy epoch, and gateway probe counters; add size and
  atomic-update tests.
- [x] Add the 1024-entry LRU in-flight operation map, x86_64 fixed-argument
  extraction, begin/end uprobe programs, bounded cleanup, duration threshold,
  and request-correlated evidence emission.
- [x] Add transactional runtime attachment for the marker pair, capability
  bits/snapshot updates, optional versus required behavior, and partial-attach
  rollback.
- [x] Extend evidence decoding and statistics tests for gateway request ID,
  route/outcome detail, PID/TID semantics, threshold, and saturation.
- [x] Harden lifecycle classification and add positive and negative gateway
  stall cases.
- [x] Update README, gateway/runtime-observability docs, FR-048 status,
  capability manifest, and executable Trellis quality contract.
- [ ] Run focused tests, full local quality gates, live Zenoh tests when the
  router is installed, then push and require the GitHub Actions run to pass.

## Focused validation

```bash
cargo fmt --all -- --check
cargo test -p esop-zenoh-gateway --features zenoh
cargo test -p esop-ebpf-runtime
cargo test -p esop-ebpf-agent
make bpf-syntax
make -C bpf clean all
```

## Full validation

```bash
make ci
make test-zenoh
git diff --check
```

After push, inspect the workflow run for the exact commit and require every BPF,
Rust, manifest, report, and Zenoh job to succeed.

## Review gates

1. Marker entry/end symmetry is reviewable independently of BPF attachment.
2. C/Rust ABI sizes and event layout pass before probe attachment is enabled.
3. Optional attachment failure cannot alter baseline capability readiness.
4. Classification rejects synthetic enum-only evidence without measured
   over-threshold duration and transport-risk correlation.
5. Documentation does not claim target-kernel attach, injection, ROS2,
   recorder, or production-overhead qualification.

## Risky files and rollback points

- `bpf/esop_runtime.bpf.c` and `bpf/vmlinux.h`: verifier/build risk; retain the
  prior tracepoint programs and keep new maps/programs additive.
- `crates/esop-ebpf-runtime/src/linux.rs`: attachment and ABI risk; keep gateway
  target attachment explicit and outside default load behavior.
- `crates/esop-zenoh-gateway/src/runtime.rs`: async cancellation risk; the guard
  must own exactly one end transition and remain feature-gated.
- `crates/esop-ebpf-agent/src/lib.rs`: safety-policy risk; add rejection tests
  before changing the positive classification path.

If a gate fails, revert only the failing increment while preserving existing
kernel evidence support. Do not relax thresholds, capability requirements, or
tests to make the build pass.
