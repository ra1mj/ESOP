# eBPF scheduler runqueue runtime qualification

## Goal

Qualify the existing FR-048/EBPF-003 scheduler runqueue-latency observation
path on a privileged hosted Linux kernel. The evidence must prove that the
production CO-RE object can load and require both scheduler tracepoints,
observe one exact-TID futex wakeup that is deliberately held runnable behind a
higher-priority single-CPU blocker, measure the resulting wake-to-switch
latency, correlate it with a risk cycle, and project the controlled-stop
incident into observer health.

## Background

The repository already implements typed `sched_wakeup` and `sched_switch`
programs, an exact scheduler-TID filter, a fixed-capacity wakeup timestamp map,
the fixed 96-byte `SchedulerRunqueueLatency` evidence ABI, per-CPU wakeup/stall
statistics, and the `HostSchedulerStall` correlation policy. Source tests and
CO-RE compilation do not establish real tracepoint attachment, exact target
wakeup identity, a measured over-threshold wake-to-schedule interval, ringbuf
delivery, or end-to-end observer-health projection.

## Requirements

- Add a Linux-only `scheduler_runqueue_qualification` example that loads the
  production CO-RE object with enabled and required masks both exactly equal to
  `ATTACH_SCHED_WAKEUP | ATTACH_SCHED_SWITCH`; unrelated programs stay disabled.
- Discover the process's real allowed CPU set with `sched_getaffinity` and
  require two distinct CPU IDs representable by the evidence ABI. Pin the
  fixture controller to CPU B and the tracked target plus blocker to CPU A.
  A single-CPU environment is a hard failure, not a skipped qualification.
- Create an exact target thread that reports its Linux TID, confirms CPU A,
  then blocks in a process-private futex wait. Confirm the task is sleeping
  before attaching/tracking it so setup activity cannot enter the evidence
  window.
- Create a blocker thread on CPU A that successfully enters `SCHED_FIFO` at a
  valid nonzero priority, reports readiness without blocking, and performs no
  allocation or syscall in its bounded spin interval. Failure to acquire the
  requested scheduler policy is a hard prerequisite failure.
- Publish the same deadline/WKC/DC-risk `CycleContext` to the kernel runtime and
  `RuntimeAgent`. Wake exactly one futex waiter while the blocker is active,
  require the target to remain unscheduled for the configured hold interval,
  then stop/join the blocker and require the target to run and join.
- Use a nonzero latency threshold materially below the bounded blocker hold.
  Bound setup waits, `/proc` sleep confirmation, blocker readiness, target
  completion, and ringbuf polling by elapsed deadlines. Every error path must
  release and join all threads and remove stale final/temp reports.
- Require one `KernelScheduler/SchedulerRunqueueLatency/Error` record with zero
  PID, exact target TID, destination CPU A, zero IRQ/ifindex/detail, count one,
  `observed_value == duration_ns > threshold`, the injected cycle identity,
  and a nonzero evidence ID/timestamp.
- Require one Error `HostSchedulerStall` incident with `ControlledStop`,
  confidence 70, matching identity/timing/cycle fields, one retained evidence
  item, and no dropped incident.
- Require exact isolated statistics: `wakeups = 1`, `scheduler_stalls = 1`,
  `emitted_events = 1`, and zero loss. Polling must report one record, one
  incident, and zero malformed, rejected, or newly reported lost records.
- Require the observer to start Healthy after capability projection and become
  Degraded with fault `0x45422001`, one incident, the exact scheduler attach
  pair, and zero loss after correlation.
- Atomically publish `build/ebpf_scheduler_runqueue_qualification.json` only
  after all runtime assertions pass. Add a strict allowlist validator and unit
  tests rejecting missing, unknown, mistyped, mismatched, lossy, or
  semantically inconsistent reports.
- Add Make targets and a dedicated privileged GitHub Actions job. Build BPF and
  Rust without elevation, elevate only the final fixture, validate as the
  invoking user, and upload the validated report.
- Update README, software PRD, runtime observability design, capability
  manifest, and backend quality guidance to claim only the hosted controlled
  runqueue-latency chain demonstrated by the report.

## Acceptance Criteria

- [ ] A two-CPU privileged hosted Linux run verifies production CO-RE load,
      required `sched_wakeup` and `sched_switch` attachment, exact target-TID
      filtering, futex wakeup, and bounded single-CPU FIFO contention.
- [ ] The target is proven asleep before injection, the futex wake reports one
      waiter, and the target remains unscheduled throughout the configured
      blocker hold before completing after blocker release.
- [ ] The run produces exactly one fixed runqueue-latency record and one
      correlated Error `HostSchedulerStall` with `ControlledStop`, confidence
      70, matching CPU/TID/cycle/timing fields, and one retained evidence item.
- [ ] Poll, statistics, correlator, and observer-health fields are exact and
      show one wakeup/stall/emission with zero malformed, rejected, dropped, or
      lost evidence.
- [ ] The qualification report is same-directory atomic and the validator is
      fail-closed for schema, integer ranges, attach pair, CPU/TID identity,
      scheduler prerequisites, timing, counts, incident semantics, and health.
- [ ] `make ci`, focused Rust/Python tests, BPF syntax, capability validation,
      formatting, shell syntax, Python compilation, and diff checks pass.
- [ ] The dedicated privileged GitHub Actions job passes and its downloaded
      report independently passes the repository validator.
- [ ] Documentation removes hosted runqueue latency from the open list while
      preserving production-kernel, natural-load/root-cause, scheduler tuning,
      product CPU-isolation, overhead/WCET, pressure, and long-duration limits.

## Non-Goals

- Do not change the BPF context, statistics, evidence ABI, discriminants,
  attach masks, incident policy, MLG authority, or realtime control path.
- Do not claim that the injected FIFO blocker represents a production root
  cause, ordinary CFS behavior, IRQ/softirq pressure, priority inversion, or
  product scheduler configuration.
- Do not qualify production kernels, target CPU isolation, realtime priority
  policy, overhead/WCET, sustained pressure, or long-duration HIL.
- Do not add a fallback that weakens the proof when `SCHED_FIFO`, two CPUs,
  tracepoints, root/passwordless sudo, or exact counts are unavailable.

## Technical Notes

- The target uses a process-private futex so the injector can require a return
  value of one from `FUTEX_WAKE_PRIVATE`. `/proc/self/task/<tid>/stat` is polled
  after target readiness to prove it entered a sleeping state before the
  blocker is started.
- The controller remains on CPU B so it can release the CPU-A FIFO blocker.
  The target stays under a singleton CPU-A affinity mask and normal scheduling;
  the FIFO blocker therefore prevents target execution until release without
  requiring realtime policy on the target itself.
- The report describes the measured interval as wake-to-switch runqueue
  latency under the injected fixture. It does not generalize the measurement
  to the product's normal workload or infer a cause beyond the controlled test.
