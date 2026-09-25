# Implementation plan

1. Rename the existing interrupt context reserve fields, add unfiltered config
   defaults/constants plus an atomic update API, and apply CPU/vector checks in
   hard-IRQ and softirq entry probes without changing map/event ABI sizes.
2. Key `HOST_IRQ_STORM` merging by evidence kind, CPU, and vector. Add unit tests
   for filter preservation/update and same-versus-distinct interrupt incidents.
3. Add the Linux-only softirq qualification example with allowed/quiet CPU
   selection, affinity verification, loopback UDP GSO injection, bounded socket
   draining, calibration/formal runtime phases, exact assertions, health
   projection, cleanup, and atomic report publication.
4. Add the closed-schema validator and Python regression tests for valid input
   plus filter/attach, CPU/vector, GSO, calibration, timing, count, loss,
   incident/cycle, and observer-health failure families.
5. Add Make targets and a dedicated privileged Actions job that installs clang,
   runs the qualification, validates it, and uploads the report.
6. Update README, FR-048, the eBPF observability status/matrix, capability
   manifest evidence/limits, and backend quality guidance with the exact hosted
   loopback softirq claim and remaining hard-IRQ/production limits.
7. Run focused and full quality gates, review the diff for ABI drift or overbroad
   claims, commit and push, wait for Actions, download the artifact, revalidate
   it locally, record measured evidence, archive the task, and push bookkeeping.

## Validation commands

```text
cargo fmt --all -- --check
cargo test -p esop-ebpf-agent
cargo test -p esop-ebpf-runtime --lib
cargo build -p esop-ebpf-runtime --example softirq_qualification
cargo clippy -p esop-ebpf-runtime --example softirq_qualification -- -D warnings
python3 -m unittest discover -s scripts/tests -p 'test_ebpf_softirq_qualification.py'
python3 -m py_compile scripts/validate-ebpf-softirq-qualification.py scripts/tests/test_ebpf_softirq_qualification.py
sh -n scripts/test-ebpf-softirq-runtime.sh
make bpf-syntax
python3 scripts/validate-capability-manifest.py
make ci
git diff --check
make test-ebpf-softirq-runtime
```

## Risk and rollback points

- Do not treat a loopback send return as proof that the softirq ran. Require all
  datagrams, the target CPU/vector evidence, and exact BPF statistics.
- Keep calibration and formal phases in separate BPF runtimes and agents. Reuse
  would contaminate counters, incidents, health, and ringbuf state.
- Apply the filter before map insertion. Filtering only at exit can leak start
  entries and corrupt later duration pairing.
- Preserve the all-CPU/all-vector defaults and the 176-byte context ABI. A
  default filter that narrows observation would be a compatibility regression.
- Do not merge hard IRQ and softirq evidence solely because vector numbers are
  equal. Interrupt class, CPU, and vector are all part of incident identity.
- Bound CPU sampling, socket I/O, polling, and report publication. On any error,
  remove stale temp/final reports and close sockets through RAII.
- Do not describe the calibration-derived threshold as a product budget or the
  loopback GSO burst as NIC, EtherCAT, production pressure, or WCET evidence.

