# Implementation plan

1. Add the Linux-only scheduler-migration qualification example with affinity
   discovery, synchronized runnable worker, exact-TID runtime update, shared
   risk cycle, two forced migrations, bounded polling, full assertions, and
   atomic report publication.
2. Add the strict report validator and Python regression tests for the valid
   schema plus attach, CPU path, identity, count, timing, loss, classification,
   cycle, and observer-health failure cases.
3. Add Make targets to run the privileged fixture and include validator tests
   in `make ci`; add a dedicated GitHub Actions job that installs clang, runs
   the qualification, and uploads the report.
4. Update README, FR-048, the eBPF observability qualification matrix/status,
   capability manifest evidence/limitations, and backend quality guidance with
   the exact hosted-kernel claim and remaining limits.
5. Run formatting, focused Rust build/tests, validator tests, BPF syntax,
   capability validation, `make ci`, and `git diff --check`. Review the final
   diff for ABI or claim expansion.
6. Commit and push the implementation. Require all GitHub Actions jobs to pass,
   download the scheduler-migration artifact, re-run the validator locally,
   record exact evidence values, then complete and archive the Trellis task and
   push the bookkeeping commits.

## Validation commands

```text
cargo fmt --all -- --check
cargo build -p esop-ebpf-runtime --example scheduler_migration_qualification
python3 -m unittest discover -s scripts/tests -p 'test_ebpf_scheduler_migration_qualification.py'
make bpf-syntax
python3 scripts/validate-capability-manifest.py
make ci
git diff --check
make test-ebpf-scheduler-migration-runtime
```

## Risk and rollback points

- A sleeping worker may have affinity changed without a visible task migration;
  keep it runnable and acknowledge actual execution on each destination CPU.
- Tracking before the initial CPU-A pin would count setup activity; enable exact
  worker-TID tracking only after readiness.
- Do not accept `>=` counts. The isolated fixture must produce exactly two
  migrations, one threshold event, one emitted record, and one incident.
- Always stop and join the worker before returning from success or failure;
  never leave a privileged busy thread behind.
- Do not hard-code CPU 0/1 or assume the host affinity mask is contiguous.
- Do not claim scheduler latency, affinity correctness, migration cause, or
  cache/NUMA impact from this event.
