# Implementation plan

1. Tighten `PageFault` classification and add threshold/cycle-risk tests in
   `esop-ebpf-agent`.
2. Extend runtime configuration, context/stat ABIs, atomic page-fault policy
   updates, decoder tests, validation, and saturating aggregation.
3. Add the typed fallback tracepoint record, bounded page-fault window map,
   threshold-only emission, and statistics to the BPF bundle.
4. Update README, observability design, software PRD, and backend quality
   guidance with the implemented boundary and remaining target qualification
   gap.
5. Run focused Rust tests, `make bpf-syntax`, `make ci`, `git diff --check`,
   and a full diff review.
6. Commit and push the implementation, require GitHub Actions CO-RE and Rust
   jobs to pass, then complete the PRD checklist, archive the task, record the
   session, and push the final metadata commits.

## Risk and rollback points

- Generated kernel BTF names the record from its event class, so the program
  must use the actual `trace_event_raw_exceptions` type and the fallback header
  must match its fixed field order.
- Keep the 96-byte ring-buffer record stable; C/Rust size checks and CI
  compilation are release gates.
- Page-fault policy fields are written as one context value so the kernel does
  not observe a partially updated threshold/window pair.
- Do not label architecture error-code bits as major/minor fault results.

## Validation commands

```text
cargo test -p esop-ebpf-agent
cargo test -p esop-ebpf-runtime
make bpf-syntax
make ci
git diff --check
```
