# eBPF process and OOM lifecycle attribution

## Goal

Make Linux lifecycle hard-fact evidence identify the affected component rather
than the thread that happened to execute a tracepoint. Ordinary worker-thread
termination must not latch a component-exit incident, and an OOM incident must
carry the PID selected by the kernel OOM killer.

## Background

The current `sched_process_exit` handler filters by the current thread group
ID but emits `ProcessExit` for every thread in that group. Because
`ProcessExit` is a 100-percent-confidence hard fact, a normal worker-thread
shutdown can become `USER_COMPONENT_EXIT` and fail the observation lease.

The current `oom:mark_victim` handler ignores its typed tracepoint record and
uses `bpf_get_current_pid_tgid()`. Linux emits `mark_victim` with the selected
task's `pid`; the current execution context may instead be the task invoking
the OOM killer. This can suppress a tracked victim or assign a critical OOM
incident to the wrong process.

## Requirements

- Treat `sched_process_exit` as thread-level input. Apply the existing
  `tracked_pid` filter to the current thread group, but emit `ProcessExit` only
  when the exiting thread ID equals the thread-group ID.
- Count tracked non-leader exits in a dedicated per-CPU diagnostic statistic
  and do not emit a hard-fact ring-buffer record for them.
- Read `oom:mark_victim` through a typed tracepoint context and use its positive
  `pid` field for filtering and evidence identity.
- In diagnostic all-process mode (`tracked_pid == 0`), emit OOM evidence for
  every positive victim PID. With a configured `tracked_pid`, emit only an
  exact victim-PID match.
- Store the selected victim PID in both fixed evidence `pid` and `tid` because
  the tracepoint exposes one task PID and not a separate thread-group ID.
- Preserve the fixed 96-byte `RuntimeEvidence` ring-buffer ABI and all existing
  evidence discriminants.
- Extend the C/Rust per-CPU statistics ABI with the ignored worker-thread exit
  counter, preserving field order for existing counters and saturating
  userspace aggregation.
- Keep OOM and leader-exit evidence as hard facts that do not require a
  correlated cycle, retain critical severity and `LatchFault`, and prove that
  the victim identity reaches the incident.
- Update README, software PRD, runtime observability design, capability
  manifest, and backend quality guidance with the implemented semantics and
  remaining qualification limits.

## Acceptance Criteria

- [x] A tracked worker-thread exit increments `thread_exits_ignored` without
      emitting `ProcessExit`; a tracked leader exit retains the existing hard
      fact path.
- [x] The OOM BPF program uses `trace_event_raw_mark_victim.pid` rather than
      the tracepoint executor's PID/TID.
- [x] Fixed evidence decoding preserves the OOM victim PID/TID and produces a
      critical `HOST_OOM` incident with 100-percent confidence and
      `LatchFault` without cycle context.
- [x] C and Rust agree on the expanded statistics size, and aggregation tests
      cover saturation of the new counter.
- [x] The real CO-RE BPF object compiles in GitHub Actions against runner BTF.
- [x] `make ci` passes locally.
- [x] Documentation distinguishes leader-exit evidence from arbitrary thread
      exit and records that exact tracked-PID OOM filtering can miss a kernel
      victim selected as another thread of the same process.

## Non-Goals

- Do not add a new thread-to-process tracking map or a `sched_process_fork`
  attach point in this increment.
- Do not claim that leader-thread exit proves every thread in the process has
  terminated; it is the single bounded lifecycle signal available from the
  current attach set.
- Do not infer OOM victim thread-group ID, exit code, signal, cgroup, memory
  pressure cause, or allocation trigger from fields absent from the tracepoint.
- Do not make eBPF evidence write MLG state, PDOs, CiA 402 controlwords, or
  motion permits.
- Do not claim target-kernel verifier, OOM-pressure injection, process-exit
  injection, ring-buffer pressure, overhead, or long-duration qualification
  from source compilation and unit tests.
