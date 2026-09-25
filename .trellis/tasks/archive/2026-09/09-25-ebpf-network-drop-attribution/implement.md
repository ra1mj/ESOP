# Implementation plan

1. Extend the fixed evidence detail field and incident tests for interface-
   scoped network threshold correlation.
2. Extend Rust runtime configuration, kernel context/stat ABIs, network update
   API, decode tests, validation, and saturating aggregation.
3. Add safe CO-RE kernel-field reads, fallback BTF types, the bounded network
   window map, protocol/ifindex filtering, and threshold emission to the BPF
   bundle.
4. Update README, capability manifest, observability design, software PRD, and
   backend quality guidance with the implemented boundary and remaining target
   qualification gap.
5. Run focused Rust tests, `make bpf-syntax`, `make ci`, `git diff --check`, and
   a full diff review.
6. Commit and push the implementation, require GitHub Actions CO-RE and Rust
   jobs to pass, then complete the PRD checkbox, archive the task, record the
   session, and push the final metadata commits.

## Risk and rollback points

- CO-RE reads through an skb pointer are verifier-sensitive. Keep the program
  optional and use `bpf_probe_read_kernel` rather than direct nested pointer
  dereference.
- Do not change the 96-byte ring-buffer record size; C/Rust size tests and CI
  compilation are release gates.
- Network filters and thresholds are updated as one context-map write so the
  kernel never observes a partially changed policy.

## Validation commands

```text
cargo test -p esop-ebpf-agent
cargo test -p esop-ebpf-runtime
make bpf-syntax
make ci
git diff --check
```
