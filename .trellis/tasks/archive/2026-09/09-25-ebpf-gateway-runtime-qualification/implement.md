# Implementation Plan: eBPF gateway runtime qualification

## Ordered Work

1. Add the gateway crate as an example-only runtime dependency and implement a
   Linux qualification executable that loads the object, requires both gateway
   probe pairs, drives two delayed marker lifecycles, polls evidence, asserts
   merge/stats semantics, and atomically writes JSON.
2. Add an exact-schema Python validator and regression tests for valid and
   malformed/cross-field-invalid reports.
3. Add a shell/Make entry point that builds unprivileged, elevates only fixture
   execution, validates the artifact, and fails clearly without privilege.
4. Add a dedicated GitHub Actions qualification job and artifact upload.
5. Update README, runtime/gateway/PRD status, known limitations where relevant,
   and the backend quality contract without broadening the claim.
6. Run focused Rust/Python checks, formatting/lint/full CI, BPF syntax and real
   CO-RE build, plus Zenoh integration; use GitHub Actions as authoritative
   privileged execution evidence.
7. Review claims, mark acceptance criteria, commit and push, monitor every CI
   job, archive the task, record the journal, and push bookkeeping commits.

## Validation Commands

```bash
cargo test -p esop-ebpf-runtime --all-targets
python3 -m unittest discover -s scripts/tests \
  -p 'test_ebpf_gateway_qualification.py'
python3 scripts/validate-ebpf-gateway-qualification.py \
  build/ebpf_gateway_qualification.json
make bpf-syntax
make -C bpf clean all
make ci
make test-zenoh
make test-ebpf-gateway-runtime
```

The final target may fail locally when passwordless sudo is absent. The
dedicated GitHub Actions job must execute it successfully and is the
authoritative privileged evidence for AC6-AC8.

## Review Gates

- Confirm the fixture links production symbols from `esop-zenoh-gateway` and
  adds no duplicate marker definitions or test-only BPF path.
- Confirm both pairs are required transactionally and all unrelated tracepoint
  attachment is disabled.
- Confirm only the fixture execution is elevated; Cargo/BPF compilation remains
  under the workspace user.
- Confirm the same transport-risk cycle is visible to kernel and agent before
  either delayed operation.
- Confirm distinct request IDs, valid route/outcome combinations, bounded delay
  and polling, and exact two-record/one-merged-incident semantics.
- Confirm report publication occurs only after runtime assertions and the
  validator rejects bool-as-int, unknown fields, partial masks, wrong details,
  duplicate IDs, timing inconsistency, loss, and mismatches.
- Confirm CI never converts unsupported verifier, attach, or ring-buffer
  behavior into a skip.
- Confirm docs distinguish direct-marker qualification from live Zenoh,
  transport causes, production kernels, and performance qualification.

## Risk And Rollback Points

- PIE symbol resolution may vary by toolchain; exact marker presence remains a
  hard requirement and must be fixed at the link/fixture boundary.
- Hosted runner BPF policy may reject load or uprobe attachment; diagnose the
  exact failure rather than weakening the job.
- Scheduler noise may change measured duration but not semantics; use a wide
  delay/threshold margin and bounded retry, never an unbounded wait.
- Ring-buffer ordering must preserve operation order. If evidence order proves
  nondeterministic, identify records by request ID in the fixture while keeping
  exact two-record assertions; do not relax route/outcome validation.
- If qualification requires production gateway or BPF behavior changes, stop
  and redesign. This task should qualify the existing observation contract.
