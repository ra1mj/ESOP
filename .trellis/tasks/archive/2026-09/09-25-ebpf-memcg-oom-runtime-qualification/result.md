# Result: eBPF memcg OOM runtime qualification

Date: 2026-09-25

## Delivered

- Added a Linux-only controlled cgroup-v2 OOM qualification fixture that
  prewarms a single-threaded child, moves only that PID into a unique leaf,
  applies a 32 MiB hard memory limit, injects a bounded 128 MiB anonymous
  mapping, and requires `SIGKILL` plus successful leaf cleanup.
- Loaded the production CO-RE object with only `ATTACH_OOM_KILL` enabled and
  required, then verified the exact victim PID through fixed evidence,
  Critical `HostOom` incident correlation, and Healthy-to-Failed observation
  projection.
- Added a closed-schema report validator, six regression tests, Make targets,
  a dedicated privileged GitHub Actions job, artifact upload, capability
  manifest evidence, PRD/design text, README guidance, and backend quality
  contracts.
- Preserved the narrow claim: this qualifies one hosted single-process memcg
  victim-PID chain, not global OOM, victim TGID, namespace/cgroup attribution,
  trigger/root cause, containers, restart policy, production kernels, or
  overhead/WCET.

## Verification

- Work commit: `560e85dda668fd930f9ab2a93653ca573e36afba`
- GitHub Actions run: `36134613286`
- Run URL: `https://github.com/ra1mj/ESOP/actions/runs/36134613286`
- All 10 jobs passed, including `ebpf-oom-runtime` and the full Rust/Zenoh
  quality job.
- Downloaded artifact:
  `esop-ebpf-oom-runtime-qualification/ebpf_oom_qualification.json`
- The downloaded report independently passed
  `scripts/validate-ebpf-oom-qualification.py`.
- Local `make ci`, focused Rust check/Clippy, six validator tests, BPF syntax,
  capability-manifest validation, shell/Python syntax, formatting, and
  `git diff --check` passed.

## Hosted measurements

- cgroup v2 memory controller enabled; leaf membership changed from exactly
  one child to zero and cleanup succeeded.
- `memory.max=33554432`, mapping `134217728`, page size `4096`, swap limit `0`,
  and group OOM disabled.
- Child PID/TID `7956` terminated with signal `9` (`SIGKILL`).
- Local counter deltas: `max=35`, `oom=1`, `oom_kill=1`,
  `oom_group_kill=0`.
- Attach masks were exactly `16` (`ATTACH_OOM_KILL`).
- BPF/poll results: one emitted OOM record, one incident, zero process exits,
  zero loss, zero malformed/rejected records, and zero dropped incidents.
- Incident policy: Critical `HostOom`, `LatchFault`, confidence `100`, no cycle
  or transition correlation.
- Final observation: Failed, one incident, zero loss, fault `0x45422002`,
  heartbeat sequence `2`.

## Residual limits

The result does not establish victim TGID, PID namespace or cgroup identity,
allocation trigger/cause, global/system OOM behavior, delegated container
behavior, swap workloads, component restart supervision, production target
kernels, sustained pressure, overhead, WCET, or long-duration HIL.
