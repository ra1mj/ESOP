# Implementation plan

1. Tighten `CpuThrottle` classification and add below-floor/cycle-risk tests in
   `esop-ebpf-agent`.
2. Extend runtime attach masks, configuration, context/stat ABIs, atomic
   frequency-policy updates, decoder tests, validation, and saturating
   aggregation.
3. Add the fallback tracepoint type, bounded policy state map, transition-only
   BPF producer, explicit policy-CPU emission, and statistics.
4. Update README, capability manifest, software PRD, runtime observability
   design, and backend quality guidance with the implemented boundary.
5. Run focused Rust tests, `make bpf-syntax`, `make ci`, `git diff --check`,
   and a full diff review. Build a real BPF object locally when available.
6. Commit and push the implementation, require GitHub Actions CO-RE and Rust
   jobs to pass, then complete the task checklist, archive it, record the
   session, and push the metadata commits.

## Risk and rollback points

- The tracepoint reports cpufreq policy values in kHz and a representative
  policy CPU; do not relabel either as instantaneous frequency or complete CPU
  membership.
- Preserve existing field offsets by appending context/stat fields only.
- The policy event may execute on a different CPU, so the emitter must accept
  an explicit affected CPU while preserving all existing producers.
- Optional-program load or attach failure must not break the default required
  observation path.
- Do not emit repeatedly while a policy remains below the floor; recovery is
  the only rearm boundary.

## Validation commands

```text
cargo test -p esop-ebpf-agent
cargo test -p esop-ebpf-runtime
make bpf-syntax
make ci
git diff --check
```
