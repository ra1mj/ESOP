# Implementation plan

1. Add the Linux-only OOM qualification example with cgroup v2 preflight,
   unique leaf lifecycle, single-threaded exec child, fixed protocol, bounded
   anonymous-memory pressure, exact wait/counter assertions, BPF correlation,
   observer health projection, cleanup, and atomic report publication.
2. Add the closed-schema validator and Python regression tests for valid input
   plus cgroup policy/counters, child membership/termination, attach mask,
   victim identity, statistics, incident/loss, cleanup, and health failures.
3. Add Make targets and a dedicated privileged Actions job that installs
   clang, runs the fixture, validates the report, and uploads the artifact.
4. Update README, FR-048/EBPF-005 runtime qualification text, capability
   manifest evidence/limits, and backend quality guidance with the exact hosted
   memcg claim and remaining semantic/production limits.
5. Run focused and full quality gates, review the diff for ABI drift or broad
   claims, commit and push, wait for Actions, download the artifact, revalidate
   it locally, record measured evidence, archive the task, and push
   bookkeeping.

## Validation commands

```text
cargo fmt --all -- --check
cargo test -p esop-ebpf-runtime --lib
cargo build -p esop-ebpf-runtime --example oom_qualification
cargo clippy -p esop-ebpf-runtime --example oom_qualification -- -D warnings
python3 -m unittest discover -s scripts/tests -p 'test_ebpf_oom_qualification.py'
python3 -m py_compile scripts/validate-ebpf-oom-qualification.py scripts/tests/test_ebpf_oom_qualification.py
sh -n scripts/test-ebpf-oom-runtime.sh
make bpf-syntax
python3 scripts/validate-capability-manifest.py
make ci
git diff --check
make test-ebpf-oom-runtime
```

## Risk and rollback points

- Never lower a limit on the parent or runner cgroup and never change global VM
  policy. The leaf must contain exactly the child before injection.
- Keep the child single-threaded so exact raw victim PID filtering has an
  unambiguous expected identity.
- Bound the anonymous mapping at four times the hard limit. Do not implement an
  unbounded allocation loop.
- Require independent cgroup kill evidence and exact BPF evidence; neither
  alone proves the full injection-to-incident chain.
- Treat `oom >= 1` as a lower bound and `oom_kill == 1` as the exact kill
  contract. Do not assume all cgroup memory-event counters advance once.
- Require `SIGKILL`, empty membership, and successful cgroup removal. Any
  different exit or cleanup residue invalidates qualification.
- Do not describe the raw victim PID as a separately proven TGID or cgroup
  identity, and do not generalize a memcg OOM into global pressure behavior.
