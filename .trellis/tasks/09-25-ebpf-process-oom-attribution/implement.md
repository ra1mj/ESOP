# Implementation plan

1. Refactor the BPF fixed-event emitter to accept explicit task identity while
   preserving all existing producer behavior and the 96-byte event ABI.
2. Gate `sched_process_exit` on `tid == tgid`, append and update the ignored
   thread-exit statistic, and read OOM victim identity from the typed
   `mark_victim` record.
3. Extend Rust statistic layout/aggregation and add decoder plus correlator
   regression tests for victim identity and hard-fact policy.
4. Update README, capability manifest, software PRD, runtime observability
   design, and backend quality guidance with the lifecycle attribution rules
   and qualification boundary.
5. Run focused Rust tests, `make bpf-syntax`, `make ci`, `git diff --check`,
   and a full diff review.
6. Commit and push the implementation, require GitHub Actions CO-RE and Rust
   jobs to pass, then complete the PRD checklist, archive the task, record the
   session, and push the metadata commits.

## Risk and rollback points

- The fallback raw OOM record must use the same `pid` field name as generated
  kernel BTF so CO-RE relocation remains explicit.
- Preserve existing statistic order and append only; C/Rust size assertions
  and saturation tests are release gates.
- Do not replace the raw victim PID with current TGID or claim a TGID that the
  tracepoint does not provide.
- Do not broaden critical OOM incidents to unrelated processes when
  `tracked_pid` is configured.

## Validation commands

```text
cargo test -p esop-ebpf-agent
cargo test -p esop-ebpf-runtime
make bpf-syntax
make ci
git diff --check
```
