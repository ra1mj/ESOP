# Implementation plan

1. Add the Linux-only scheduler-runqueue qualification example with allowed-CPU
   discovery, controller/target affinity, private futex synchronization,
   sleeping-state confirmation, bounded FIFO blocker, exact-TID runtime load,
   shared risk cycle, full runtime assertions, RAII cleanup, and atomic report.
2. Add a strict report validator and Python regression tests for valid schema
   plus attach-pair, scheduler prerequisite, CPU/TID, futex, timing, count,
   classification, cycle, loss, and observer-health failure families.
3. Add Make targets to run the privileged fixture and include report tests in
   `make ci`; add a dedicated GitHub Actions job that installs clang, runs the
   qualification, and uploads the report.
4. Update README, FR-048, the eBPF observability qualification status/matrix,
   capability-manifest evidence/limitations, and backend quality guidance with
   the exact controlled hosted-kernel claim and remaining limits.
5. Run formatting, focused Rust build/Clippy, validator tests, BPF syntax,
   capability validation, Python/shell syntax, `make ci`, and diff checks.
   Review the final diff for ABI changes or overbroad scheduler claims.
6. Commit and push the implementation. Require all GitHub Actions jobs to pass,
   download the runqueue artifact, re-run the validator locally, record exact
   evidence values, then archive the Trellis task and push bookkeeping commits.

## Validation commands

```text
cargo fmt --all -- --check
cargo build -p esop-ebpf-runtime --example scheduler_runqueue_qualification
cargo clippy -p esop-ebpf-runtime --example scheduler_runqueue_qualification -- -D warnings
python3 -m unittest discover -s scripts/tests -p 'test_ebpf_scheduler_runqueue_qualification.py'
python3 -m py_compile scripts/validate-ebpf-scheduler-runqueue-qualification.py scripts/tests/test_ebpf_scheduler_runqueue_qualification.py
sh -n scripts/test-ebpf-scheduler-runqueue-runtime.sh
make bpf-syntax
python3 scripts/validate-capability-manifest.py
make ci
git diff --check
make test-ebpf-scheduler-runqueue-runtime
```

## Risk and rollback points

- A readiness flag set immediately before futex wait is not sufficient proof
  that the target is sleeping. Confirm the target task state before starting
  the FIFO blocker, and require the futex wake return value to be exactly one.
- The controller must stay runnable on CPU B; pinning it with the target and
  FIFO blocker on CPU A would prevent bounded release and deadlock the fixture.
- Do not perform allocation, channel operations, sleeping, or yielding inside
  the FIFO blocker after it reports ready. Keep the hold far below ordinary
  Linux realtime throttling intervals and bound every parent-side wait.
- Do not accept `>=` event counts. The isolated fixture must produce exactly
  one wakeup, one scheduler stall, one emitted record, and one incident.
- Always release and join target/blocker threads on failure. A privileged
  realtime spinner must never outlive the qualification process cleanup path.
- Do not describe the injected wake-to-switch interval as product WCET,
  ordinary scheduler performance, priority inversion, or production root cause.
