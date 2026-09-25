# Design: eBPF scheduler migration runtime qualification

## Qualification data flow

```text
host allowed CPU set
  -> worker pins itself to CPU A and reports Linux TID
  -> production BPF object attaches only sched_migrate_task
  -> runtime atomically tracks worker TID, threshold=2, bounded window
  -> identical WKC-risk cycle published to BPF context and RuntimeAgent
  -> parent replaces worker singleton affinity A -> B -> A
  -> worker acknowledges execution on each destination CPU
  -> BPF count window emits only the second migration
  -> Aya fixed-record decoder -> RuntimeAgent correlator -> degraded heartbeat
  -> exact JSON report -> strict validator -> CI artifact
```

## Worker and affinity protocol

- Read the current process affinity with `sched_getaffinity(0, ...)` and select
  the first two allowed CPUs that fit `u16`. This uses the runner/cgroup's real
  allowance rather than assuming CPU 0 and CPU 1 are available.
- Spawn one worker with shared atomics and an acknowledgement channel. The
  worker obtains `gettid`, pins itself to CPU A, confirms `sched_getcpu() == A`,
  and reports ready before the runtime tracks it.
- The worker stays runnable in a bounded spin/yield loop. For each requested
  target, the parent stores the desired CPU, applies a singleton affinity mask
  to the worker TID, and waits for an acknowledgement that the worker actually
  executed on that CPU. A stop command is always issued and the thread joined.
- Tracking starts only after the initial A pin is complete. Singleton masks
  leave no alternate CPU for load balancing, so the only tracked moves are the
  parent-controlled A-to-B and B-to-A replacements.

## Runtime contract

- Load with `enabled_attach_mask == required_attach_mask ==
  ATTACH_SCHED_MIGRATE_TASK`. Assert runtime and capability-snapshot masks and
  `attach_ready()` before injection.
- Use a nonzero latency threshold only to satisfy the complete scheduler policy
  contract; the run does not enable wakeup/switch programs. Set migration
  threshold to two and a window large enough for bounded synchronization.
- Call `update_scheduler_tracking(worker_tid, ..., 2, window_ns)` after worker
  readiness so the policy epoch invalidates any earlier state. Publish one
  WKC-risk cycle with fixed cycle and transition IDs to both runtime and agent.
- Poll until the single expected record/incident arrives or the deadline
  expires. Exact counts are required because every unrelated attach point is
  disabled and only the worker TID is tracked.

## Evidence, incident, and health contracts

- The first A-to-B move creates count-window state and increments migration
  statistics without emitting. The second B-to-A move crosses the threshold,
  yielding one Warning evidence item with `pid=0`, worker TID, `irq=B`,
  `cpu=A`, count/observed/threshold equal to two, positive bounded duration,
  and the injected cycle identity.
- Correlation produces one Warning `HostSchedulerStall`, action
  `DegradeHostObservation`, confidence 60, and one retained evidence item.
  The incident is not described as measured stall latency.
- Capability projection initially makes the agent Healthy. The incident moves
  it to Degraded with fault `0x45422001`; loss remains zero.

## Report and validation

- The example deletes stale final/temp files and writes a same-directory temp
  report followed by rename only after every assertion succeeds.
- The JSON schema is a closed allowlist. It includes selected CPUs, worker TID,
  policy thresholds, attach masks, poll/statistics fields, full incident and
  evidence identity, and initial/final observation state.
- The validator rejects booleans as integers, values outside unsigned ABI
  ranges, non-distinct CPUs, any mismatch in the A-to-B-to-A endpoint, wrong
  cycle/count/classification, nonpositive or out-of-window duration, loss, and
  unknown or missing fields. Unit tests exercise each failure family and the
  CLI path.

## Compatibility and safety

- This task adds host qualification only; production BPF and Rust event ABIs
  remain unchanged. The example is Linux-only and uses the existing `libc`
  dev-dependency.
- BPF/Rust compilation and Python validation run unprivileged. Root or
  passwordless sudo is required only for the already-built fixture that loads
  BPF and changes affinity inside its own process.
- The worker cannot write MLG, PDO, controlword, or motion-permit state. Its
  failure aborts qualification and cannot create a successful report.

## Operational limits and rollback

- Hosted CI proves one kernel/runner configuration with at least two allowed
  CPUs. It does not qualify product CPU isolation, realtime priorities,
  scheduler tuning, production kernel versions, latency, or overhead.
- A single-CPU runner fails with an explicit prerequisite error. CI must not
  convert that condition into a skip.
- Rollback removes the example, script, validator/tests, Make/CI targets, and
  documentation claims; no runtime ABI or product-state migration is needed.
