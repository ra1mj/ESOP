# Implementation plan

1. Add the Linux-only network-drop qualification example with privilege/tool
   preflight, unique veth lifecycle, CPU affinity, protocol-zero AF_PACKET
   injection, negative controls, exact BPF correlation/health assertions,
   bounded cleanup, and atomic report publication.
2. Add the closed-schema validator and Python regression tests for valid input
   plus link identity/configuration, controls, attach masks, packet/drop counts,
   statistics, evidence, incident/loss, cleanup, and health failures.
3. Add Make targets and a dedicated privileged Actions job that installs
   clang/iproute2, runs the fixture, validates the report, and uploads the
   artifact.
4. Update README, FR-048/EBPF-004 runtime qualification text, capability
   manifest evidence/limits, and backend quality guidance with the exact hosted
   virtual-veth claim and remaining semantic/production limits.
5. Run focused and full quality gates, review the diff for ABI drift or broad
   claims, commit and push, wait for Actions, download the artifact, revalidate
   it locally, record measured evidence, archive the task, and push bookkeeping.

## Validation commands

```text
cargo fmt --all -- --check
cargo test -p esop-ebpf-runtime --lib
cargo build -p esop-ebpf-runtime --example network_drop_qualification
cargo clippy -p esop-ebpf-runtime --example network_drop_qualification -- -D warnings
python3 -m unittest discover -s scripts/tests -p 'test_ebpf_network_drop_qualification.py'
python3 -m py_compile scripts/validate-ebpf-network-drop-qualification.py scripts/tests/test_ebpf_network_drop_qualification.py
sh -n scripts/test-ebpf-network-drop-runtime.sh
make bpf-syntax
python3 scripts/validate-capability-manifest.py
make ci
git diff --check
make test-ebpf-network-drop-runtime
```

## Risk and rollback points

- Own only uniquely named disposable veth interfaces and delete them on every
  exit path. Never modify a physical interface, route, firewall, qdisc, or
  host-wide network setting.
- Open the AF_PACKET socket with protocol zero so the injector does not become
  a receive handler for the protocol it is trying to prove unhandled.
- Keep control and formal phases separate. Capture the formal interface-counter
  baseline after controls while requiring exact zero BPF counters throughout
  the controls.
- Pin to one inherited CPU because network aggregation is per CPU. Failure to
  set or restore affinity invalidates qualification.
- Require independent interface-drop evidence and exact protocol/ifindex-
  filtered BPF evidence; neither alone proves the full injection-to-incident
  chain.
- Preserve the kernel reason as bounded nonzero detail without hard-coding an
  enum value whose numbering may vary by kernel.
- Require runtime detach, socket close, link deletion, sysfs disappearance,
  and affinity restoration. Any residue invalidates qualification.
- Do not generalize one hosted virtual receive-drop path into physical NIC,
  driver, queue-pressure, production-kernel, overhead, or WCET coverage.
