# Design: eBPF scheduler runqueue runtime qualification

## Qualification data flow

```text
host allowed CPU set
  -> controller pins itself to CPU B
  -> target pins to CPU A, reports Linux TID, enters private futex sleep
  -> production BPF object attaches only sched_wakeup + sched_switch
  -> exact target TID and risk cycle are active in kernel and RuntimeAgent
  -> CPU-A blocker enters SCHED_FIFO and bounded spin
  -> controller wakes exactly one target futex waiter from CPU B
  -> target remains runnable but cannot execute during blocker hold
  -> controller releases blocker; sched_switch selects exact target TID
  -> BPF measures wake-to-switch latency and emits one fixed record
  -> Aya decoder -> RuntimeAgent correlator -> degraded heartbeat
  -> atomic JSON report -> strict validator -> CI artifact
```

## Thread and scheduling protocol

- Read the current process affinity and select the first two allowed CPUs that
  fit `u16`. Pin the controller to CPU B before creating qualification threads.
- The target thread sets singleton CPU-A affinity, verifies `sched_getcpu`,
  reports its TID, and waits on an `AtomicU32` using `FUTEX_WAIT_PRIVATE` while
  its value remains zero. The controller confirms `/proc/self/task/<tid>/stat`
  reports sleeping before loading the exact-TID policy.
- The blocker inherits CPU B, moves to CPU A, enters `SCHED_FIFO` using a valid
  priority obtained from the host, marks an atomic ready flag, then spins only
  on an atomic stop flag. It performs no channel send, allocation, sleep, or
  yield after realtime policy is active.
- The controller calls `FUTEX_WAKE_PRIVATE` after storing one to the target
  gate and requires exactly one waiter to be woken. It waits for the configured
  hold interval and asserts the target completion flag is still false before
  releasing and joining the blocker. Target completion and join then prove the
  delayed runnable entity eventually executed.
- RAII drop paths store the stop/release flags, wake the futex, and join any
  remaining thread so a failed privileged fixture cannot leave a FIFO spinner.

## Runtime contract

- Load with `enabled_attach_mask == required_attach_mask ==
  ATTACH_SCHED_WAKEUP | ATTACH_SCHED_SWITCH` and `scheduler_tid` equal to the
  sleeping target TID. Assert runtime/snapshot masks and `attach_ready()`.
- Use a latency threshold below the blocker hold; unrelated scheduler migration
  policy values remain valid but no migration program is loaded.
- Publish one fixed risk cycle to both runtime and agent before the futex wake.
  Baseline relevant statistics and ringbuf state must be empty.
- Poll after target completion until one record and incident arrive or the
  deadline expires. Exact counts are required because only the exact target TID
  can update the wakeup map and only the scheduler pair is attached.

## Evidence, incident, and health contracts

- The BPF wakeup hook stores `bpf_ktime_get_ns` under the target TID and counts
  one wakeup. The switch hook reads `next_pid`, deletes the timestamp, and emits
  only when the measured interval is strictly greater than the configured
  threshold.
- Evidence is Error `KernelScheduler/SchedulerRunqueueLatency`, with
  `pid=0`, `tid=target`, `cpu=CPU A`, zero IRQ/ifindex/detail, count one, and
  identical positive `observed_value` and `duration_ns` above threshold. This
  producer passes a zero explicit evidence ID, so the shared emitter writes the
  same nonzero emit timestamp to `evidence_id` and `timestamp_ns`.
- Correlation produces one Error `HostSchedulerStall`, action `ControlledStop`,
  confidence 70, and one retained evidence item. Capability projection first
  makes the agent Healthy; the incident changes it to Degraded with fault
  `0x45422001`. This observation remains diagnostic and does not directly
  control motion.

## Report and validation

- Delete stale final/temp reports at process start. Write the full report to a
  same-directory temporary path and rename it only after every assertion.
- The closed schema records selected CPUs, target TID, FIFO priority, threshold
  and hold interval, futex wake count, target-during-hold state, attach masks,
  relevant baseline/final statistics, poll fields, full incident/evidence, and
  initial/final observation state.
- The validator rejects booleans as integers, unknown/missing fields, invalid
  CPUs/TID/priority, partial attach pairs, zero or multiple futex wakes, a target
  that ran during the hold, duration/threshold inconsistencies, wrong cycle or
  classification, count drift, loss, and observation mismatches.

## Compatibility and safety

- The implementation is a Linux-only example and reuses the runtime crate's
  existing `libc` development dependency. Production BPF and Rust ABIs remain
  unchanged.
- Hosted validation exposed scheduler-migration fixture setup noise: loading
  that separate fixture with `scheduler_tid=0` briefly tracked its process
  leader before the worker TID was known. Its setup now uses the valid but
  unreachable `i32::MAX` TID until the existing atomic exact-TID update, so
  runner background migration cannot contaminate the exact-zero baseline.
- BPF/Rust compilation and report validation remain unprivileged. Only the
  prebuilt fixture is elevated for BPF loading, affinity, futex, and FIFO
  scheduling operations.
- A missing capability, invalid host scheduler range, denied FIFO transition,
  single-CPU runner, or count mismatch produces no successful report.

## Operational limits and rollback

- Hosted CI proves one controlled wake-to-switch measurement on its kernel and
  runner. It does not qualify normal product workloads, product priority plans,
  CPU isolation, PREEMPT_RT behavior, sustained contention, overhead, or WCET.
- Rollback removes the example, script, validator/tests, Make/CI targets, and
  documentation claims. No runtime ABI or persistent-state migration exists.
