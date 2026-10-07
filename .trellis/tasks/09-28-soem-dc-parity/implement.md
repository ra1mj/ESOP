# Implementation plan

## Readiness-audit dependencies

The [readiness audit](../../../docs/v1-readiness-audit.md#ordered-v1-closure)
owns the complete family/sample matrix and current binary-test evidence.
Before enabling maintenance writes or file transfers, provide single-NIC
runtime ownership and bounded local maintenance admission. Mailbox protocols
depend on a shared fair broker, not independent per-protocol RX owners. ENI
depends on normalized product configuration. Redundancy depends on shared RX
duplicate authority and one authoritative DC sample. Complete API/CLI/process
and daemon/package evidence before software RC; then qualify the frozen product
and target with physical hardware. The ordered steps below must carry these
dependencies into each child artifact.

## Protocol implementation sequence

1. Complete and archive the runtime diagnostics CLI child so every later
   capability has stable status and incident visibility.
2. Create a cyclic-mailbox child task covering fixed fairness, protocol
   demultiplexing, scheduler ownership, and DC/P0 isolation.
3. Add FoE as the first optional protocol with maintenance-mode product policy,
   bounded transfer state, progress diagnostics, and resource impact report.
4. Add SoE IDN access and mapping discovery through the shared mailbox layer.
5. Add EoE host-domain fragmentation/reassembly and bounded traffic budgets.
6. Add dual-port redundancy below core RX admission with exact generation,
   duplicate/reorder, WKC and DC reference tests.
7. Add supervised recovery policy and execute the two-vendor drive plus IO HIL
   matrix without automatic motion rearm.
8. For every slice update requirements, product configuration, capability
   manifest, build/performance evidence and backend specifications; run
   `make ci`, commit and push independently.

## Per-slice validation

```text
cargo test -p esop-ethercat-core
cargo test -p esop-ethercat-linux-port --all-features
make capability-manifest
make performance-report
make ci
git diff --check
```

Each slice additionally needs a deterministic regression that runs due Domain
frames, cyclic DC and the new service together and proves unchanged P0 frame
plans, exact WKC and bounded completion.
