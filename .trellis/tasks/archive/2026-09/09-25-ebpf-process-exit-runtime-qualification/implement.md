# Implementation Plan: eBPF process-exit runtime qualification

## Ordered Work

1. Add a Linux qualification example with synchronized parent/child modes,
   process-exit-only loading, tracked-PID switch, worker suppression checks,
   leader hard-fact assertions, health projection checks, bounded polling, and
   atomic JSON publication.
2. Add an exact-schema Python validator and regression tests for valid and
   malformed/cross-field-invalid process-exit reports.
3. Add a minimal-privilege shell/Make entry point that builds unprivileged,
   elevates only fixture execution, validates the artifact, and fails clearly
   without privilege.
4. Add a dedicated GitHub Actions qualification job and artifact upload.
5. Update README, FR-048, runtime observability design, capability manifest,
   and backend quality contract without broadening the claim.
6. Run focused Rust/Python checks, formatting/lint/full CI, BPF syntax and real
   CO-RE build; use GitHub Actions as authoritative privileged evidence.
7. Review claims, mark acceptance criteria, commit and push, monitor every CI
   job, archive the task, record the journal, and push bookkeeping commits.

## Validation Commands

```bash
cargo test -p esop-ebpf-runtime --all-targets
python3 -m unittest discover -s scripts/tests \
  -p 'test_ebpf_process_exit_qualification.py'
python3 scripts/validate-ebpf-process-exit-qualification.py \
  build/ebpf_process_exit_qualification.json
make bpf-syntax
make -C bpf clean all
make ci
make test-ebpf-process-exit-runtime
```

The final target may fail locally when passwordless sudo is absent. The
dedicated GitHub Actions job must execute it successfully and is the
authoritative privileged evidence for AC8-AC9.

## Review Gates

- Confirm the fixture enables and requires only `ATTACH_PROCESS_EXIT` and uses
  the production BPF object, decoder, statistics, correlator, and health path.
- Confirm the child cannot create the worker thread before the parent updates
  `tracked_pid`, and the parent cannot exit before report publication.
- Confirm the worker acknowledgement follows `join`, then prove no record,
  incident, or failed lease before releasing the leader.
- Confirm the leader evidence uses child PID for both PID/TID, has no invented
  exit cause or cycle attribution, and produces the existing hard-fact policy.
- Confirm only fixture execution is elevated; Cargo/BPF compilation and report
  validation remain under the workspace user.
- Confirm JSON publication occurs only after runtime assertions and the
  validator rejects bool-as-int, unknown fields, partial masks, wrong counts,
  false worker incidents, identity/semantic drift, loss, and health mismatch.
- Confirm CI never converts unsupported verifier, attach, or ring-buffer
  behavior into a skip.
- Confirm docs distinguish leader-exit qualification from complete process
  death, exit-cause attribution, OOM, restart supervision, and performance.

## Risk And Rollback Points

- Process startup/exit sequencing must use pipe acknowledgements, not assumed
  scheduler timing.
- The parent initially tracks itself to avoid global noise, then switches once
  to the waiting child before any child thread termination.
- A worker exit must be observed through the statistics map before the leader
  release; otherwise fail rather than weakening the assertion.
- If hosted CI exposes unexpected extra exits for the tracked child, inspect
  the child runtime and protocol instead of relaxing exact counts.
- No production ABI change is expected. Any required BPF or correlator behavior
  change triggers redesign rather than being hidden inside qualification code.
