# Implementation plan

1. Tighten `AgentHealth` so event loss has a dedicated fault, remains sticky
   within one epoch, and restart clears all epoch-local capability/counter/fault
   state before a new snapshot may restore Healthy.
2. Add unit and cross-layer regression coverage for capability classifications,
   same-epoch loss stickiness, restart reset, stale-epoch rejection, and MLG
   fail-closed behavior.
3. Add the Linux/x86_64 degradation qualification example with production
   page-fault-only load, bounded anonymous first-write batches, no-poll
   ring-buffer saturation, exact loss projection, runtime unload/reload, epoch
   recovery, cleanup, and atomic report publication.
4. Add the closed-schema validator and Python mutation tests for capability
   policy, attach masks, saturation bounds/statistics, loss/heartbeat
   relationships, restart reset, recovery, and cleanup.
5. Add Make targets and a dedicated privileged Actions job that builds the
   production BPF object, runs the fixture, validates the report, and uploads
   the artifact.
6. Update README, FR-047/FR-050, EBPF-008, capability evidence/limitations, and
   backend quality guidance with the exact hosted claim and remaining limits.
7. Run focused and full quality gates, commit and push, wait for Actions,
   download/revalidate the artifact, record measured evidence, archive the
   task, and push bookkeeping.

## Validation commands

```text
cargo fmt --all -- --check
cargo test -p esop-ebpf-agent
cargo test -p esop-procbuf --test cross_layer
cargo test -p esop-ebpf-runtime --lib
cargo build -p esop-ebpf-runtime --example observability_degradation_qualification
cargo clippy -p esop-ebpf-runtime --example observability_degradation_qualification -- -D warnings
python3 -m unittest discover -s scripts/tests -p 'test_ebpf_observability_degradation_qualification.py'
python3 -m py_compile scripts/validate-ebpf-observability-degradation-qualification.py scripts/tests/test_ebpf_observability_degradation_qualification.py
sh -n scripts/test-ebpf-observability-degradation-runtime.sh
make bpf-syntax
python3 scripts/validate-capability-manifest.py
make ci
git diff --check
make test-ebpf-observability-degradation-runtime
```

## Risk and rollback points

- Keep the production 4 MiB ring buffer unchanged; a smaller test map would
  not qualify the shipped behavior.
- Bound memory resident at one chunk and bound total touched pages. Fail rather
  than loop indefinitely when loss does not occur.
- Do not poll before kernel loss is observed, because polling would invalidate
  the saturation experiment.
- Treat incidental page faults as allowed noise; validate monotonic positive
  relationships rather than an exact fault count.
- Drop runtime 1 before restart and load runtime 2 with the new epoch so old
  maps/records cannot masquerade as recovery.
- Preserve Failed precedence over event-loss degradation and preserve the
  lifecycle guard's inability to treat Degraded as motion-qualified.
- Do not present synthetic capability snapshots as actual missing-BTF or
  permission runtime qualification.
