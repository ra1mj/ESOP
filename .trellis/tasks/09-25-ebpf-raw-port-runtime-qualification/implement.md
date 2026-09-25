# Implementation Plan: eBPF raw-port runtime qualification

## Ordered Work

1. Add runtime-crate example dependencies and a Linux qualification executable
   that loads the object, requires the raw-port pair, injects a delayed blocking
   `recv(2)`, polls evidence, asserts incident/stats, and atomically writes JSON.
2. Add an exact-schema Python validator and regression tests for valid and
   malformed/cross-field-invalid reports.
3. Add a reusable shell/Make entry point that builds unprivileged, elevates only
   execution, validates the artifact, and fails clearly without privilege.
4. Add a dedicated GitHub Actions qualification job and artifact upload.
5. Update the capability manifest, runtime-observability design, README/status
   claim where applicable, and backend quality contract.
6. Run focused Rust/Python tests, formatting/lint/full CI, BPF syntax and real
   CO-RE build locally; use GitHub Actions as the authoritative privileged run.
7. Review claims, mark acceptance criteria, commit/push, monitor all jobs,
   archive the task, record the journal, and push the bookkeeping commits.

## Validation Commands

```bash
cargo test -p esop-ebpf-runtime --all-targets
python3 -m unittest discover -s scripts/tests -p 'test_ebpf_raw_port_qualification.py'
python3 scripts/validate-ebpf-raw-port-qualification.py \
  build/ebpf_raw_port_qualification.json
make bpf-syntax
env LD_LIBRARY_PATH=/home/mj/.local/opt/clang-14/usr/lib/x86_64-linux-gnu \
  make -C bpf clean all CLANG=/home/mj/.local/opt/clang-14/usr/bin/clang-14
make ci
make test-ebpf-raw-port-runtime
```

The final target is expected to fail locally when passwordless sudo is absent;
the dedicated GitHub Actions job must execute it successfully and is the
authoritative privileged evidence for AC5-AC7.

## Review Gates

- Confirm the fixture exact-links production marker symbols and does not add a
  test hook or indirect syscall to `LinuxRawPort`.
- Confirm only the fixture execution is elevated; Cargo and BPF compilation run
  as the workspace user.
- Confirm kernel tracepoint attachment is disabled for isolation and the full
  raw-port pair is required.
- Confirm cycle risk exists in both kernel context and agent before injection.
- Confirm the delayed operation is an actual `recv(2)`, result/error is captured
  before end, and polling/timeouts are bounded.
- Confirm the report is published only after all runtime assertions and the
  validator rejects bool-as-int, unknown fields, partial pairs, timing
  inconsistency, loss, mismatches, and wrong incident semantics.
- Confirm CI does not silently skip unsupported BPF or uprobe behavior.
- Confirm docs distinguish shared marker/syscall qualification from AF_PACKET,
  hardware, overhead, and production realtime qualification.

## Risk And Rollback Points

- Hosted runner BPF policy may reject loading or uprobe attachment; treat that
  as evidence and diagnose the exact permission/verifier failure rather than
  weakening the job to a skip.
- PIE symbol resolution may differ by toolchain; exact marker symbol presence
  remains a hard failure and must be fixed in the fixture/link contract.
- Poll timing can be noisy; use a wide delay/threshold margin and bounded retry,
  never lower semantic assertions or add an unbounded sleep.
- If the fixture requires production-path changes, stop and redesign; this task
  must qualify existing behavior without altering the realtime port path.
