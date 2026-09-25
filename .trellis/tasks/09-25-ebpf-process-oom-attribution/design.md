# Design: eBPF process and OOM lifecycle attribution

## Data flow

```text
sched:sched_process_exit
  -> current tgid/tid
  -> tracked tgid filter
  -> tid != tgid: diagnostic counter only
  -> tid == tgid: fixed ProcessExit evidence
  -> USER_COMPONENT_EXIT hard fact

oom:mark_victim
  -> typed raw tracepoint pid
  -> positive/exact tracked victim filter
  -> fixed evidence with pid == tid == victim pid
  -> HOST_OOM hard fact
```

## Kernel contracts

- Add `trace_event_raw_mark_victim` to the fallback `vmlinux.h`; generated
  runner/target BTF remains authoritative for CO-RE compilation and loading.
- Split fixed evidence emission into an explicit-task helper accepting `pid`
  and `tid`. The existing helper delegates with either the current task IDs or
  zeros, preserving scheduler, page-fault, IRQ, softirq, and network behavior.
- `sched_process_exit` reads current IDs once. A matching non-leader increments
  `thread_exits_ignored` and returns. A matching leader emits the existing
  process hard fact and increments `process_exits`.
- `oom:mark_victim` reads the signed tracepoint PID, rejects non-positive
  values, and applies `tracked_pid` to that victim PID. It emits with explicit
  victim identity and increments `oom_events`.

## ABI and userspace

- The ring-buffer record remains 96 bytes.
- `KernelStats` appends `thread_exits_ignored`, growing the C/Rust map value
  from 120 to 128 bytes without shifting existing counters.
- Per-CPU aggregation uses saturating addition for the new counter.
- OOM records set both `pid` and `tid` to the raw victim PID. This is explicit
  uncertainty rather than inventing a thread-group ID.

## Correlation and safety

- `OomKill` and `ProcessExit` remain hard facts: cycle correlation is not
  required, and the resulting incident remains Critical with `LatchFault` and
  100-percent confidence.
- Suppressing tracked non-leader exits prevents routine thread-pool lifecycle
  from failing the observation lease.
- eBPF and the agent remain observation-only; the existing supervisor lease
  and MLG policy retain final control authority.

## Compatibility and rollback

- Existing statistic offsets remain stable because the new field is appended.
- Existing BPF program names, attach masks, evidence kinds, map names, and
  96-byte decoder remain unchanged.
- Rollback removes the appended statistic and explicit OOM identity helper;
  no ProcBuf, EtherCAT, MLG, or external schema migration is required.

## Qualification boundary

The `mark_victim` tracepoint exposes `task->pid`, not a separate TGID. Exact
`tracked_pid` filtering therefore cannot prove membership when the kernel
selects another thread from the same process. Diagnostic all-process mode
retains those records. Resolving that gap requires another bounded identity
source or a different hook and is outside this increment.

Local tests prove decoder, incident, and statistics contracts. GitHub CI proves
CO-RE source compilation against its runner BTF. Target-kernel loading,
verifier behavior, process/OOM injection, PID-namespace behavior, ring-buffer
pressure, runtime overhead, and long-duration stability remain EBPF-005,
EBPF-010, and EBPF-011 environment evidence.
