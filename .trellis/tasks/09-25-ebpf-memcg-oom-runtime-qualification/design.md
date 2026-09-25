# Design: eBPF memcg OOM runtime qualification

## Qualification data flow

```text
cgroup v2 root preflight
  -> create unique leaf, set memory.max/swap/oom.group policy
  -> exec single-threaded child, warm fixed control path, report ready
  -> move exact child PID into leaf and verify sole membership
  -> production BPF object attaches only oom:mark_victim for child PID
  -> verify zero BPF and memory.events.local baseline + Healthy heartbeat
  -> release child -> bounded anonymous page faults -> memcg OOM -> SIGKILL
  -> memory.events.local confirms local OOM kill
  -> ringbuf decode -> HostOom hard fact -> Failed heartbeat
  -> detach BPF -> verify empty leaf -> remove cgroup
  -> atomic JSON report -> strict validator -> CI artifact
```

## Cgroup isolation contract

- The fixture requires the unified cgroup v2 hierarchy at `/sys/fs/cgroup`.
  Both the root controller list and subtree control must expose `memory`.
- The parent creates `/sys/fs/cgroup/esop-oom-qualification-<parent-pid>` with
  mode inherited from the hierarchy. A pre-existing path is a hard failure.
- The leaf receives a fixed 32 MiB `memory.max`. `memory.swap.max` is set to
  zero when present and `memory.oom.group` is explicitly set to zero.
- Only the prepared child PID is written to `cgroup.procs`. The fixture reads
  membership back and requires exactly one PID before release.
- Baseline local memory-event counters are captured after the move and before
  release. Because the leaf is new, `oom`, `oom_kill`, and `oom_group_kill`
  must all be zero.
- After the child dies, require `oom_kill` delta exactly one,
  `oom_group_kill` delta zero, and `oom` delta at least one. The kernel may
  increment `oom`/`max` more than once while resolving one kill, so those
  counters are not required to equal one.

## Child isolation protocol

- The same executable supports `--oom-child`. It remains single-threaded and
  uses fixed stack buffers for the ready/release protocol.
- Startup calls the exact pipe-read and anonymous-mapping helpers once before
  readiness, then unmaps the warm-up region. The parent moves the blocked child
  into the leaf only after receiving readiness.
- On release, the child maps 128 MiB anonymously and writes one byte per system
  page. This is bounded at four times the 32 MiB limit and is expected to be
  killed before completion. Reaching the end, returning an allocation error,
  or exiting normally is a qualification failure.
- Parent deadlines cover readiness and termination. RAII kills/reaps an
  incomplete child. The expected wait status is `SIGKILL`.

## Runtime and correlation contract

- The runtime enables and requires only `ATTACH_OOM_KILL`, with
  `tracked_pid=child_pid`. Existing production BPF, decoder, statistics, and
  correlator code remain unchanged.
- OOM is a hard fact and does not require `CycleContext`; cycle and transition
  fields remain zero throughout evidence and incident projection.
- The BPF program reads the raw victim PID and writes it as both PID and TID.
  The fixture's child is single-threaded, so exact tracked-PID filtering must
  accept the event without claiming a separate TGID observation.
- One OOM record produces Critical `HostOom`, action `LatchFault`, confidence
  100. Runtime health changes from Healthy to Failed with the latched-incident
  fault code `0x45422002`.

## Evidence and statistics contract

- Evidence domain/kind/severity are KernelMemory/OomKill/Critical. CPU is the
  hook execution CPU and is only range-checked and matched between evidence and
  incident; no CPU affinity claim is made.
- PID/TID equal the child PID. IRQ, ifindex, cycle, transition, duration, and
  detail are zero. Observed value, threshold, and count are one.
- Exact BPF statistics are one OOM event, one emitted event, zero process exits,
  and zero loss. Polling yields one record and one incident with no malformed or
  rejected evidence and no dropped incident.

## Cleanup and report publication

- Delete stale report and temporary paths before setup.
- On success or failure, ensure the child has been reaped. If the leaf remains
  populated, write `1` to `cgroup.kill` when available and wait for
  `cgroup.events` to report `populated 0`.
- Explicit success cleanup requires empty `cgroup.procs` and successful leaf
  removal. Cleanup failure prevents report publication.
- Only after cleanup succeeds, write the complete report to a same-directory
  temporary path and rename it atomically.
- The closed schema records cgroup mode/path policy (without treating the path
  as stable identity), limit/mapping sizes, membership, local event baselines
  and deltas, child wait status, attach masks, BPF stats, full incident/evidence
  fields, and initial/final health.

## Compatibility and safety

- This task adds a qualification harness only. Production BPF maps, programs,
  fixed ABIs, runtime APIs, and incident policy are unchanged.
- The example is Linux-only and uses existing `libc` support. Production crates
  gain no dependency.
- BPF/Rust compilation and report validation remain unprivileged. Only the
  final prebuilt fixture is elevated for cgroup and BPF operations.

## Operational limits and rollback

- Hosted CI proves one controlled cgroup-v2 single-process OOM victim chain.
  It does not qualify global OOM, victim TGID/cgroup/namespace attribution,
  trigger/root-cause analysis, containers, restart policy, production kernels,
  overhead, or WCET.
- Rollback removes the example, runner, validator/tests, Make/CI targets, docs,
  and capability evidence entry. Production runtime behavior is unaffected.
