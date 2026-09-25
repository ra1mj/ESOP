# Implementation plan

1. Add the Linux-only page-fault qualification example with allowed-CPU
   selection, exec-child preparation, fixed binary control/results, anonymous
   page injection, exact baseline/final assertions, correlation, health
   projection, cleanup, and atomic report publication.
2. Add the closed-schema validator and Python regression tests for valid input
   plus architecture/page geometry, attach/window, child/CPU/cycle identity,
   count/statistics, detail/timing, incident, loss, and observer-health failure
   families.
3. Add Make targets and a dedicated privileged Actions job that installs clang,
   runs the fixture, validates the report, and uploads the artifact.
4. Update README, FR-048/EBPF-005 runtime qualification text, capability
   manifest evidence/limits, and backend quality guidance with the exact hosted
   anonymous-page claim and remaining semantic/production limits.
5. Run focused and full quality gates, review the diff for ABI drift or broad
   claims, commit and push, wait for Actions, download the artifact, revalidate
   it locally, record measured evidence, archive the task, and push bookkeeping.

## Validation commands

```text
cargo fmt --all -- --check
cargo test -p esop-ebpf-runtime --lib
cargo build -p esop-ebpf-runtime --example page_fault_qualification
cargo clippy -p esop-ebpf-runtime --example page_fault_qualification -- -D warnings
python3 -m unittest discover -s scripts/tests -p 'test_ebpf_page_fault_qualification.py'
python3 -m py_compile scripts/validate-ebpf-page-fault-qualification.py scripts/tests/test_ebpf_page_fault_qualification.py
sh -n scripts/test-ebpf-page-fault-runtime.sh
make bpf-syntax
python3 scripts/validate-capability-manifest.py
make ci
git diff --check
make test-ebpf-page-fault-runtime
```

## Risk and rollback points

- Do not load BPF until the child has completed every setup/warm-up operation
  and is blocked. Any tracked setup fault invalidates the exact count contract.
- Keep the child pinned throughout the tracked phase. Migration would split the
  existing `(CPU,TGID)` aggregation and can suppress the threshold crossing.
- Use fixed raw buffers after readiness. Rust formatting, allocation, thread
  creation, or buffered stdio in the tracked phase can add unrelated faults.
- Require both the child's exact `ru_minflt` delta and exact BPF stats; neither
  one alone proves the full injection-to-evidence chain.
- Treat detail 6 and `ru_minflt` as hosted x86_64 fixture facts only. Do not
  generalize them into portable major/minor classification.
- Bound child readiness/result/exit and BPF polling. On any error, remove stale
  temp/final reports and reap the child.
- Do not describe count-window elapsed time as fault-handler duration or the
  fixture threshold as a product budget.
