# Design: eBPF CPU migration attribution

## Data flow

```text
sched:sched_migrate_task
  -> typed {pid, prio, orig_cpu, dest_cpu} read
  -> scheduler TID filter
  -> bounded per-TID count window + policy epoch
  -> first migration-count threshold crossing
  -> fixed 96-byte RuntimeEvidence
  -> Aya decoder
  -> IncidentCorrelator + transport-risk CycleContext
  -> lower-confidence HOST_SCHEDULER_STALL
```

## Kernel contracts

- Add a 1024-entry LRU hash keyed by scheduler TID. Its value stores the
  monotonic window start, saturating migration count, scheduler-policy epoch,
  and the latest origin/destination CPUs.
- Append scheduler TID, migration threshold, policy epoch, reserved padding,
  and migration window to the context ABI. The scheduler filter uses
  `scheduler_tid` when nonzero, otherwise the existing `tracked_pid`; two zero
  values retain the all-task development mode.
- A missing entry, expired window, backwards timestamp, or epoch mismatch
  starts a new count window at one. The producer emits only when the previous
  count was below the threshold and the new count reaches it.
- Invalid PID/CPU records are ignored. Map update failure increments
  `lost_events` and emits nothing because bounded window state was not retained.
- Append valid migration and threshold-event counters to statistics. Existing
  offsets remain unchanged.
- Change runqueue-latency emission to pass the tracepoint `next_pid` explicitly
  as TID and leave PID zero. This removes current-task misattribution at the
  `sched_switch` hook.

## ABI and userspace

- Append `CpuMigration = 10`; do not move evidence or incident discriminants.
- The fixed record remains 96 bytes. For migration evidence:
  - `pid = 0`, `tid = sched_migrate_task.pid`
  - `cpu = dest_cpu`, `irq = orig_cpu`
  - `observed_value = count`, `threshold = configured count threshold`
  - `duration_ns = elapsed time in the active count window`
  - `count = count`, `detail = saturated scheduler priority`
- Add one optional attach bit and `AttachPoint` entry. It is enabled by default
  but remains outside the default required mask.
- `RuntimeConfig` validates the count/window and builds a context epoch of one.
  `update_scheduler_tracking` validates a complete replacement context before
  the map write, increments the epoch, and commits local state only after the
  kernel map accepts the update.

## Correlation and safety

- `CpuMigration` requires a nonzero threshold, nonzero count at or above that
  threshold, and a matching deadline/WKC/DC-risk cycle. It maps to the existing
  `HostSchedulerStall` incident with warning severity, host-observation
  degradation, and lower confidence than measured runqueue latency.
- Incident merging continues to use TID, so migration and measured runqueue
  latency for the same scheduler entity can form one bounded timeline while
  different tracked TIDs remain separate.
- The secondary `irq` field is interpreted as origin CPU only inside raw
  `CpuMigration` evidence; top-level incident consumers must branch on the
  evidence kind before naming it an interrupt vector.
- No observation directly changes MLG, PDO, controlword, or permit state.

## Compatibility and rollback

- Existing evidence and incident layouts and discriminants do not move.
- Context and statistics fields are append-only; C/Rust size assertions and
  tests are release gates.
- Existing `tracked_pid` scheduler filtering remains effective whenever the new
  scheduler-specific TID is zero.
- Missing tracepoint support degrades only this optional capability.
- Rollback removes the optional program, attach bit, LRU map, appended fields,
  new evidence kind, and docs without changing Protobuf or ProcBuf ABI.

## Qualification boundary

Local tests prove configuration validation, fixed ABI decoding, count-window
semantics at source level, and incident correlation. CO-RE compilation proves
the typed tracepoint can relocate against build BTF. Real task-affinity and
migration injection, target-kernel verifier/load behavior, event loss, runtime
overhead, cache/NUMA impact, and long-duration stability remain environment
qualification work.
