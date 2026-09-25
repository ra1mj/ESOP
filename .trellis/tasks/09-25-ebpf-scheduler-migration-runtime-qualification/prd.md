# eBPF scheduler migration runtime qualification

## Goal

Qualify the existing FR-048/EBPF-003 scheduler-migration observation path on a
privileged hosted Linux kernel. The evidence must prove that the production
CO-RE object can load and attach `sched:sched_migrate_task`, observe exactly
two controlled migrations of one tracked scheduler entity, emit the first
count-threshold crossing, correlate it with a transport-risk cycle, and project
the resulting lower-confidence scheduler incident into observer health.

## Background

The repository already implements a typed `sched_migrate_task` producer, an
exact scheduler-TID filter, a fixed 1024-entry epoch-aware migration window,
the fixed 96-byte `CpuMigration` evidence ABI, saturating statistics, and the
`HostSchedulerStall` correlator policy. Existing source, ABI, decoder, and unit
tests do not establish target-kernel verifier/load behavior, real tracepoint
attachment, actual task migration, ring-buffer delivery, or end-to-end health
projection. Gateway, raw-port, and process-exit paths already provide the
report/validator/privileged-CI pattern this task must follow.

## Requirements

- Add a Linux-only `scheduler_migration_qualification` example that loads the
  production CO-RE object with enabled and required masks both equal to
  `ATTACH_SCHED_MIGRATE_TASK`; unrelated programs must stay disabled.
- Discover the fixture's actual allowed CPU set with `sched_getaffinity`.
  Qualification requires two distinct CPU IDs representable by the fixed
  evidence ABI; fewer than two usable CPUs is a hard failure, not a skip or a
  successful reduced claim.
- Create one synchronized worker thread, pin it to CPU A before it becomes the
  tracked scheduler entity, report its Linux TID, and keep it runnable under a
  singleton affinity mask. After `BpfRuntime::update_scheduler_tracking`, force
  and acknowledge exactly two moves, CPU A to B and B to A, within a bounded
  nonzero migration window.
- Configure a migration count threshold of two and publish the same WKC-risk
  `CycleContext` to the kernel runtime and `RuntimeAgent` before injection.
- Bound worker synchronization and ring-buffer polling by elapsed deadlines.
  Every error path must stop and join the worker; stale output and temporary
  files must be removed before execution.
- Require one `KernelScheduler/CpuMigration/Warning` record whose PID is zero,
  TID is the worker TID, origin CPU is B in the kind-specific `irq` field,
  destination CPU is A in `cpu`, count/observed value/threshold are two,
  duration is positive and inside the configured window, cycle identity is
  preserved, and scheduler-priority detail remains within its fixed byte.
- Require one Warning `HostSchedulerStall` incident with
  `DegradeHostObservation`, confidence 60, matching TID/CPU/cycle/count fields,
  one retained evidence item, and no dropped incident.
- Require exact kernel statistics for this isolated fixture:
  `scheduler_migrations = 2`,
  `scheduler_migration_threshold_events = 1`, `emitted_events = 1`, and zero
  lost events. Polling must report one record, one incident, and zero malformed,
  rejected, or newly reported lost records.
- Require the observer to start Healthy after capability projection and become
  Degraded with fault `0x45422001`, one incident, the migration attach mask,
  and zero loss after the correlated incident.
- Atomically publish `build/ebpf_scheduler_migration_qualification.json` only
  after all runtime assertions pass. Add a strict allowlist validator and unit
  tests that reject missing, unknown, mistyped, mismatched, lossy, or
  semantically inconsistent reports.
- Add Make targets and a dedicated privileged GitHub Actions job. The script
  must compile BPF and Rust without elevation, elevate only the final fixture,
  validate as the invoking user, and upload the validated report.
- Update README, software PRD, runtime observability design, capability
  manifest, and backend quality guidance to claim only the hosted-kernel
  migration chain actually demonstrated by the report.

## Acceptance Criteria

- [ ] A two-CPU hosted Linux run verifies production CO-RE load, required
      `sched_migrate_task` attachment, exact worker-TID tracking, and bounded
      A-to-B-to-A affinity-driven migration.
- [ ] The run produces exactly one fixed migration record and one correlated
      lower-confidence scheduler incident with the required source/destination,
      cycle, count, severity, action, confidence, and priority fields.
- [ ] Poll, statistics, correlator, and observer-health fields are exact and
      show zero malformed, rejected, dropped, or lost evidence.
- [ ] The qualification report is same-directory atomic and the validator is
      fail-closed for schema, integer ranges, attach masks, identity,
      migration path, counts, timestamps, incident semantics, and health.
- [ ] `make ci`, focused Rust/Python tests, BPF syntax, and diff checks pass.
- [ ] The dedicated privileged GitHub Actions job passes and its downloaded
      report independently passes the repository validator.
- [ ] Documentation removes hosted scheduler migration from the open list
      while preserving production-kernel, migration-cause, affinity-policy,
      cache/NUMA impact, latency, overhead/WCET, pressure, and long-duration
      qualification limits.

## Non-Goals

- Do not infer why Linux migrated the task or claim an affinity-policy defect.
- Do not interpret the migration-window duration as measured scheduler stall,
  execution latency, cache loss, NUMA impact, or deadline delay.
- Do not qualify runqueue latency, IRQ/softirq pressure, cpufreq limits, OOM,
  production kernels, performance overhead, WCET, or long-duration HIL here.
- Do not change the BPF event layout, evidence/incident discriminants,
  correlation policy, MLG authority, or realtime control path.

## Technical Notes

- The threshold-crossing event is the second migration, so the qualified raw
  path is origin CPU B (`irq`) to destination CPU A (`cpu`).
- A singleton CPU affinity mask prevents ordinary load balancing from adding
  unrelated migrations while the worker is tracked. The worker remains
  runnable so each affinity replacement forces an actual scheduler move.
- The report records scheduler priority as observed by the tracepoint but does
  not equate it with userspace `sched_priority` or use it as a pass/fail policy
  beyond the fixed unsigned-byte ABI.
