# Linux memcg OOM qualification research

Date: 2026-09-25

## Primary sources

- Linux cgroup v2 documentation:
  `https://docs.kernel.org/admin-guide/cgroup-v2.html`
- Linux OOM tracepoint definition:
  `https://github.com/torvalds/linux/blob/master/include/trace/events/oom.h`
- Linux OOM victim implementation:
  `https://github.com/torvalds/linux/blob/master/mm/oom_kill.c`

## Findings used by the design

- `memory.max` is the cgroup v2 hard memory limit. If usage reaches the limit
  and reclaim cannot reduce it, the OOM killer is invoked inside the cgroup.
- A cgroup-local OOM does not kill tasks outside that cgroup.
- `memory.events.local` exposes local `max`, `oom`, `oom_kill`, and
  `oom_group_kill` counters. `oom_kill` counts killed processes; `oom` may
  advance more than once around a single kill and is used only as a lower
  bound.
- `memory.oom.group=0` keeps the default per-victim behavior. The fixture still
  uses a single-process leaf to avoid ambiguous group kills.
- `TRACE_EVENT(mark_victim)` stores `task->pid` plus memory/accounting fields.
  The current ESOP ABI deliberately exports only the raw PID and does not claim
  a separate TGID, namespace, or cgroup identity.
- The kernel sends `SIGKILL` to the selected victim and calls
  `mark_oom_victim`; the qualification therefore requires both the tracepoint
  event and a `SIGKILL` child wait status.
