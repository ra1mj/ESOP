# eBPF CPU migration attribution

## Goal

Complete the bounded FR-048 scheduler-migration observation path so ESOP can
identify repeated migration of a tracked realtime or gateway scheduling entity
when it overlaps a degraded EtherCAT cycle, without treating an ordinary
single load-balancing migration as a root cause.

## Background

The runtime already measures tracked scheduler wakeup-to-run latency through
`sched_wakeup` and `sched_switch`, but it does not attach
`sched_migrate_task`. Linux exposes the migrated scheduler entity PID/TID,
priority, origin CPU, and destination CPU at that tracepoint. A migration is a
scheduler fact, not proof of delay or failure, so incident classification must
also require a configured migration-count threshold and transport-risk cycle.

## Requirements

- Observe the optional `sched:sched_migrate_task` tracepoint through its typed
  context and validate positive PID plus nonnegative, distinct origin and
  destination CPUs that fit the fixed event ABI.
- Add an explicit scheduler TID filter. Zero preserves the existing behavior by
  falling back to `tracked_pid`; if both are zero, scheduler hooks observe all
  scheduling entities. Process, OOM, and page-fault hooks continue to interpret
  `tracked_pid` as a process identifier.
- Expose a nonzero migration-count threshold and nonzero window in
  `RuntimeConfig`, the C/Rust context ABI, and an atomic runtime update method.
  Updating scheduler policy must advance an epoch so existing map state cannot
  cross a threshold under stale configuration.
- Keep one fixed-capacity LRU state entry per observed scheduler TID. Count
  migrations in a monotonic bounded window, saturate the count, emit only when
  the count first reaches the threshold, and reset on window expiry or policy
  epoch change.
- Preserve the fixed 96-byte evidence record. Write the scheduler entity to
  `tid` with `pid = 0`, the destination CPU to `cpu`, the origin CPU to the
  kind-specific secondary `irq` slot, the migration count to `observed_value`
  and `count`, the configured count threshold to `threshold`, elapsed window
  time to `duration_ns`, and saturated scheduler priority to `detail`.
- Correct scheduler runqueue-latency evidence to identify the tracepoint's
  tracked `next_pid` as `tid` rather than attributing the hook's current task.
- Append per-CPU statistics for valid tracked migrations and emitted migration
  threshold events. Rust aggregation must remain saturating.
- Add `CpuMigration` as an append-only evidence discriminant. Classify it as a
  lower-confidence `HOST_SCHEDULER_STALL` only when its count reaches the
  configured threshold and it overlaps a deadline/WKC/DC-risk cycle. A healthy
  cycle, a below-threshold event, or malformed count evidence must not create an
  incident.
- Keep the attach point optional in the default required mask so kernels
  without the tracepoint report reduced capability without breaking the
  scheduler/process baseline.
- Preserve the existing incident discriminants and the fixed ring-buffer ABI.
  Keep eBPF observation outside the realtime control path and unable to write
  MLG, PDO, controlword, or motion-permit state.
- Update README, software PRD, runtime observability design, capability
  manifest, and backend quality guidance without claiming migration cause,
  scheduler-affinity correctness, target-kernel injection, or timing impact.

## Acceptance Criteria

- [ ] The BPF bundle contains a typed `sched_migrate_task` program and a
      fixed-capacity per-TID migration-window map.
- [ ] Migration state resets on window expiry and policy epoch change, emits
      once on the threshold crossing, and cannot grow without bound.
- [ ] Fixed evidence decodes tracked TID, origin/destination CPU, count,
      threshold, window duration, and priority without changing its 96-byte
      size or existing discriminants.
- [ ] Scheduler configuration rejects zero/oversized thresholds and zero
      windows without partially mutating the kernel context.
- [ ] Runqueue-latency evidence and migration evidence both carry the actual
      tracked scheduler TID rather than the task executing the hook.
- [ ] Unit tests prove migration attribution requires both threshold evidence
      and a correlated transport-risk cycle and remains lower confidence than
      measured runqueue latency.
- [ ] C and Rust agree on expanded context/stat sizes, attach masks remain
      explicit, and statistics aggregation tests cover saturation.
- [ ] `make bpf-syntax` and `make ci` pass locally.
- [ ] GitHub Actions builds the real CO-RE BPF object and passes the Rust gate.

## Non-Goals

- Do not treat a single CPU migration as scheduler failure.
- Do not infer the reason for a migration, affinity-policy correctness,
  cache/NUMA impact, cgroup throttling, or CPU isolation quality.
- Do not add scheduler latency histograms, task runtime accounting, cgroup or
  cpuset attribution, affinity mutation, or userspace stack capture.
- Do not claim target-kernel verifier, migration injection, event-pressure,
  overhead, or long-duration qualification from source and unit tests.

## Notes

- `sched_migrate_task.pid` is the scheduler entity identifier used as the
  evidence TID. The tracepoint does not provide a trustworthy TGID, so the
  producer leaves evidence PID zero instead of inventing process attribution.
- The existing `irq` field is kind-specific in fixed evidence: it remains the
  interrupt vector for IRQ evidence and carries the origin CPU only for
  `CpuMigration`. The destination CPU remains the primary `cpu` field.
