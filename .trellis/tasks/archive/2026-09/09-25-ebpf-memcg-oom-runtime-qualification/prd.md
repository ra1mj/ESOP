# eBPF memcg OOM runtime qualification

## Goal

Qualify the existing FR-048/EBPF-005 OOM-victim path on a privileged hosted
Linux kernel without exposing the runner or unrelated processes to global
memory exhaustion. The evidence must prove that the production CO-RE object
can load and require `oom:mark_victim`, attribute one locally injected cgroup
v2 OOM kill to the exact tracked single-threaded child PID, emit the fixed OOM
record, correlate it as a hard-fact `HOST_OOM` incident, and project the
incident into failed observer health.

## Background

The repository already reads `trace_event_raw_mark_victim.pid`, filters it by
the configured tracked PID, emits a fixed 96-byte `KernelMemory/OomKill`
record, decodes that record, and classifies it as Critical `HOST_OOM` with
`LatchFault` and 100-percent confidence without requiring cycle correlation.
Source tests and CO-RE compilation do not establish real verifier/load
behavior, tracepoint availability, local OOM injection, victim identity,
ringbuf delivery, exact kernel statistics, or health projection.

Linux cgroup v2 documents `memory.max` as a hard limit that invokes the OOM
killer inside the cgroup when reclaim cannot reduce usage. A cgroup-local OOM
does not kill tasks outside that cgroup, and `memory.events.local` exposes
`oom`, `oom_kill`, and `oom_group_kill` counters. The kernel `mark_victim`
tracepoint stores `task->pid`; it does not provide a separate TGID or cgroup
identity.

## Requirements

- Add a Linux-only `oom_qualification` example that requires cgroup v2 at
  `/sys/fs/cgroup`, creates a unique leaf cgroup directly below the hierarchy
  root, configures a fixed nonzero `memory.max`, disables swap when
  `memory.swap.max` is present, and leaves `memory.oom.group` disabled.
- Spawn the same executable in a single-threaded child mode. The child must
  complete startup, raw control-pipe warm-up, and fixed-buffer preparation
  before it reports readiness and blocks. The parent must move only that exact
  PID into the qualification cgroup and verify membership before injection.
- Load the production BPF object with enabled and required attach masks both
  exactly `ATTACH_OOM_KILL`, tracked PID equal to the child PID, and no
  unrelated tracepoint or uprobe. Require a complete capability snapshot and
  a Healthy initial observer heartbeat.
- Require a zero baseline for BPF emitted/lost/OOM statistics and for the new
  cgroup's local `oom`, `oom_kill`, and `oom_group_kill` counters.
- Release the child through the prepared pipe. It must create and fault a
  bounded anonymous mapping larger than `memory.max` until the memcg OOM killer
  terminates it. The parent must require `SIGKILL`, not a normal exit, timeout,
  or unrelated signal.
- Require local cgroup evidence after the child exits: `oom >= 1`,
  `oom_kill == 1`, and `oom_group_kill == 0`. Do not interpret the number of
  `oom` or `max` events as a stable cross-kernel exact count.
- Require exactly one `KernelMemory/OomKill/Critical` record with PID/TID equal
  to the child PID, zero IRQ/ifindex/cycle/transition/duration/detail,
  `observed_value == threshold == count == 1`, and nonzero evidence identity
  and timestamp.
- Require exactly one Critical `HostOom` incident with `LatchFault`, confidence
  100, matching child identity and evidence, no dropped incident, and no cycle
  correlation. Observer health must transition from Healthy to Failed with
  fault `0x45422002`.
- Require exact final BPF statistics: `oom_events=1`, `emitted_events=1`, zero
  process-exit and lost events. Polling must report one record, one incident,
  and zero malformed, rejected, or newly reported lost records.
- Bound child readiness, OOM termination, cgroup-empty observation, BPF polling,
  and cleanup. On any failure, kill/reap the child, use `cgroup.kill` when
  available, remove the leaf cgroup, and do not publish a qualified report.
- Remove stale final/temp reports at start and atomically publish
  `build/ebpf_oom_qualification.json` only after all assertions and cgroup
  cleanup pass. Add a closed-schema validator and regression tests.
- Add Make targets and a dedicated privileged GitHub Actions job. Build BPF and
  Rust without elevation, elevate only the final fixture, validate as the
  invoking user, and upload the validated report.
- Update README, software PRD, runtime observability design, capability
  manifest, and backend quality guidance with only the controlled hosted
  single-process memcg OOM claim demonstrated by the report.

## Acceptance Criteria

- [x] A privileged hosted Linux run verifies production CO-RE load and required
      `oom:mark_victim` attachment with no unrelated hook.
- [x] A unique cgroup v2 leaf confines one prepared single-threaded child; the
      child is the only member and is terminated by `SIGKILL` after bounded
      anonymous-memory pressure exceeds the configured hard limit.
- [x] Independent `memory.events.local` evidence records at least one OOM,
      exactly one OOM kill, and no group OOM kill while the runner remains
      alive and the cgroup is removed successfully.
- [x] Baseline BPF counters are zero; the formal run produces exactly one fixed
      OOM record and exact final OOM/emission/loss statistics.
- [x] The record and incident preserve the exact victim PID/TID and hard-fact
      semantics: Critical `HostOom`, `LatchFault`, confidence 100, no cycle
      dependency, and Healthy-to-Failed heartbeat transition with fault
      `0x45422002`.
- [x] The report is same-directory atomic and its validator fails closed for
      schema, cgroup configuration/counters, child termination, attach mask,
      victim identity, event/statistics, incident policy, loss, cleanup, and
      observer-health mismatches.
- [x] Focused Rust/Python tests, formatting, Clippy, BPF syntax, capability
      validation, shell/Python syntax, `make ci`, and `git diff --check` pass.
- [x] The dedicated privileged GitHub Actions job passes and its downloaded
      artifact independently passes the repository validator.
- [x] Documentation removes controlled hosted single-process memcg OOM
      injection from the open list while preserving victim TGID, namespace,
      cgroup attribution, OOM trigger/root cause, global pressure, production
      kernel, overhead/WCET, restart supervision, and long-duration limits.

## Non-Goals

- Do not trigger global OOM, modify host-wide overcommit/sysctl policy, or place
  the qualification parent/runner in the limited cgroup.
- Do not infer victim TGID, namespace identity, cgroup ID, OOM trigger task,
  allocation size, memory-pressure cause, exit reason beyond the observed
  `SIGKILL`, or complete component restart behavior from `mark_victim`.
- Do not change the fixed event/context/map ABI, evidence discriminants, attach
  bits, incident severity/action policy, MLG authority, or realtime path.
- Do not claim `memory.events.local.oom` or `max` has an exact portable count,
  or that one hosted cgroup run qualifies global/system OOM behavior.
- Do not claim production target kernels, containers/delegated cgroups, swap
  behavior, sustained pressure, overhead, WCET, or long-duration HIL.
- Missing cgroup v2, memory controller, writable hierarchy, tracepoint, BPF
  permission, root/passwordless sudo, exact victim event, exact kill count, or
  cleanup success must fail rather than skip qualification.

## Technical Notes

- Use the cgroup v2 root rather than the fixture's current service/session
  cgroup so the memory controller is already delegated by the root hierarchy
  on the hosted runner. Verify `cgroup.controllers` and
  `cgroup.subtree_control` both contain `memory` before creating the leaf.
- Move the child after its startup pages are resident. Existing charges remain
  with the old cgroup, while the bounded formal anonymous mapping is charged to
  the new leaf and deterministically exceeds `memory.max`.
- Use raw `mmap` plus one volatile byte write per system page. The mapping size
  is a fixed multiple of the limit and remains bounded; no unbounded allocation
  loop is allowed.
- `memory.events.local.oom_kill` is independent injection evidence. The BPF
  record remains the authoritative proof that the production tracepoint path
  observed the kernel-selected victim PID.
