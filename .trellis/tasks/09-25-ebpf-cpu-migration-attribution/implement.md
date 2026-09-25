# Implementation plan

1. Add append-only `CpuMigration` decoding and lower-confidence correlated
   scheduler-stall classification with focused agent tests.
2. Extend runtime attach masks, scheduler configuration, context/stat ABIs,
   atomic policy updates, decoder tests, validation, and saturating aggregation.
3. Add the fallback tracepoint type, scheduler filter helper, bounded migration
   state map, threshold-crossing BPF producer, corrected runqueue TID emission,
   and statistics.
4. Update README, capability manifest, software PRD, runtime observability
   design, and backend quality guidance with the implemented boundary.
5. Run focused Rust tests, `make bpf-syntax`, `make ci`, `git diff --check`,
   and a full diff review. Build a real BPF object locally when available.
6. Commit and push the implementation, require GitHub Actions CO-RE and Rust
   jobs to pass, then complete the checklist, archive the task, record the
   session, and push metadata commits.

## Risk and rollback points

- `sched_migrate_task.pid` identifies a scheduler entity, not a confirmed TGID;
  evidence must not invent process attribution.
- Preserve existing field offsets by appending context/stat fields only.
- Keep origin/destination validation before narrowing to the fixed u16 slots.
- Count threshold crossings rather than every migration to bound ring-buffer
  pressure; reset stale state when runtime policy changes.
- Optional-program load or attach failure must not break the required baseline.
- A migration threshold alone is correlation evidence, not measured stall time;
  classification must remain weaker than runqueue-latency evidence.

## Validation commands

```text
cargo test -p esop-ebpf-agent
cargo test -p esop-ebpf-runtime
make bpf-syntax
make ci
git diff --check
```
