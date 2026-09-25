# Implementation plan: eBPF gateway callback stall attribution

## Ordered work

- [x] Generalize the gateway request allocator for publish and callback
  observations; add stable callback markers, outcomes, guard, and invocation
  helper with completion/unwind tests.
- [x] Wrap command subscription and query callbacks at the gateway-owned
  invocation boundary without changing public signatures or typed-query
  behavior.
- [x] Record and validate gateway operation class in the existing bounded BPF
  state; refactor publish helpers and add callback begin/end uprobe programs.
- [x] Add callback capability bits, exact symbols, probe specs, and a reusable
  transactional pair-attachment helper plus public callback attachment API.
- [x] Extend exact-symbol, decode/mask, compatibility, and live Zenoh coverage.
- [x] Update README, gateway/runtime observability docs, FR-048, capability
  manifest, and backend quality contract with the implemented boundary.
- [x] Run focused tests, full local gates, live Zenoh tests, real BPF object
  compilation, then push and require GitHub Actions success.

## Focused validation

```bash
cargo fmt --all -- --check
cargo test -p esop-zenoh-gateway --features zenoh
cargo test -p esop-ebpf-runtime -p esop-ebpf-agent
cargo clippy -p esop-zenoh-gateway -p esop-ebpf-runtime --all-targets --all-features -- -D warnings
make bpf-syntax
```

## Full validation

```bash
make ci
make test-zenoh
make -C bpf clean all
git diff --check
```

## Review gates

1. Callback wrappers preserve the exact public Fn contracts and typed-query
   reply/error behavior.
2. Publish and callback classes cannot cross-complete BPF state.
3. Optional callback attachment cannot affect kernel baseline readiness.
4. Fixed context, statistics, map, and 96-byte event ABIs do not change.
5. Documentation narrows only the query/subscribe gap and preserves all real
   target, ROS2, recorder, injection, and overhead limitations.

## Risky files and rollback points

- `crates/esop-zenoh-gateway/src/runtime.rs`: callback ownership/unwind risk;
  retain public signatures and measure only gateway-owned invocation.
- `bpf/esop_runtime.bpf.c`: verifier and shared-state risk; keep map layout
  fixed and make operation class explicit.
- `crates/esop-ebpf-runtime/src/linux.rs`: pair rollback/capability risk; use
  one helper for publish and callback attachment semantics.
- Documentation/capability files: do not imply real privileged attach or
  production overhead qualification.
