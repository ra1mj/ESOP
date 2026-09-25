# eBPF process-exit runtime qualification

## Goal

Turn the existing `sched_process_exit` source, decoder, statistics, and
correlator evidence into an executable privileged qualification. The test must
prove that the repository's actual CO-RE object can be verified, loaded, and
attached to the kernel process-exit tracepoint; that an ordinary worker-thread
exit is ignored as a component failure; and that the tracked process leader's
exit becomes one auditable `UserComponentExit` hard-fact incident and a failed
host-observation lease.

This is a hosted Linux integration test. It must not be presented as
qualification of exit code/signal attribution, complete thread-group death,
PID namespaces, cgroups, OOM behavior, ROS 2/recorder restart policy,
production target kernels, probe overhead/WCET, or long-duration operation.

## Background

- `esop_process_exit` already filters by tracked TGID, increments
  `thread_exits_ignored` for non-leader TIDs, and emits `ProcessExit` only when
  `tid == tgid`.
- `BpfRuntime` can load the real object, require only `ATTACH_PROCESS_EXIT`,
  update the tracked PID through the context map, decode the fixed 96-byte
  record, aggregate per-CPU statistics, and bridge it into `RuntimeAgent`.
- `RuntimeAgent` classifies `ProcessExit` as a Critical,
  100-percent-confidence `UserComponentExit` hard fact with `LatchFault`; it
  does not require a transport-risk cycle.
- FR-048 and EBPF-005 currently leave real process-exit injection, verifier,
  attach, ring-buffer, statistics, and incident behavior unqualified.
- GitHub's Ubuntu runner provides noninteractive sudo and is the executable
  evidence environment for this increment.

## Requirements

### R1. Privileged process lifecycle fixture

- Add a Linux-only qualification executable that loads the repository-built
  BPF object with every unrelated tracepoint and user probe disabled and
  requires only `sched:sched_process_exit`.
- Spawn the same executable in a bounded child mode that waits for explicit
  parent commands. Set the BPF tracked PID to the child's positive PID before
  allowing any child thread to terminate.
- First create, join, and acknowledge exactly one worker-thread exit while the
  child leader remains alive. Require the ignored-thread statistic to advance
  without a ring-buffer record, incident, or failed observer lease.
- Then release the child leader, require a normal child exit, and poll with
  both iteration and elapsed-time limits. Verifier/load, attach, protocol,
  timeout, malformed/rejected evidence, or missing incident is a hard failure.

### R2. Evidence, incident, health, and statistics assertions

- Require exactly one accepted record and one incident after the leader exits,
  with zero malformed/rejected/lost records and zero dropped incidents.
- Require one emitted kernel event, one `process_exits`, one
  `thread_exits_ignored`, zero `oom_events`, and zero lifetime lost events.
- Require evidence with the configured boot ID and agent epoch, nonzero
  evidence ID/timestamp, child PID in both PID/TID, zero cycle/transition,
  `KernelProcess/ProcessExit/Critical`, observed value and threshold one,
  count one, zero duration/detail/IRQ/netdev, and a bounded CPU identifier.
- Require a matching `UserComponentExit` incident with Critical severity,
  `LatchFault`, confidence 100, child PID/TID, one evidence item, count one,
  zero lost events, and no cycle or transition attribution.
- Require the observer to be Healthy after complete attachment and the ignored
  worker exit, then Failed after the leader incident. Its heartbeat must carry
  the process-exit attach bit, incident count one, zero loss, and the existing
  latched-incident fault code.

### R3. Machine-readable qualification artifact

- Write an atomic deterministic exact-schema JSON report under `build/` only
  after every runtime assertion succeeds. Failure returns nonzero and leaves
  no success artifact.
- Include schema/status, attach masks, poll counters, process/worker/OOM/loss
  statistics, incident fields, evidence identity/semantics, and final
  host-observation state/fault counters.
- Add a standalone validator that treats JSON as untrusted input, rejects
  booleans as integers, rejects unknown/missing fields, bounds unsigned values,
  and verifies every cross-field invariant from R2.
- Add regression tests for the qualified baseline plus missing/unknown/mistyped
  fields, partial/wrong attachment, false worker-exit incidents, wrong counts,
  wrong PID/TID/domain/kind/severity/action, nonzero cycle data, loss, and
  inconsistent observer state.

### R4. Reproducible local and CI entry point

- Add one Make target/script that builds the BPF object and Rust fixture as the
  workspace user, elevates only the final executable when root or passwordless
  sudo is available, and validates the artifact.
- If privilege is unavailable, fail with a precise prerequisite message; never
  silently skip or claim qualification.
- Add a dedicated GitHub Actions job that installs the BPF compiler, runs the
  qualification target, and uploads the validated report. Unsupported
  verifier/load/attach/ring-buffer behavior must fail the job.
- Include validator regression tests in the ordinary `make ci` quality gate.

### R5. Claims and safety boundary

- Update README, FR-048, the runtime observability design, capability manifest,
  and Trellis quality contract with the exact qualified boundary and artifact.
- State that the result proves the shared leader-exit-to-incident and observer
  lease path on hosted Linux, including suppression of one controlled worker
  exit. It does not prove why a process exited or that all threads are gone.
- Keep OOM pressure, exit code/signal, PID namespace/cgroup identity,
  ROS 2/recorder restart semantics, production kernels, performance/WCET, and
  long-duration HIL explicitly open.
- The qualification path must not write lifecycle state, controlword, motion
  permit, command page, or production configuration.

## Acceptance Criteria

- [x] AC1: The fixture loads the real CO-RE object with only the required
  process-exit tracepoint and switches tracking to a synchronized child PID
  before allowing any child thread to terminate.
- [x] AC2: One controlled worker-thread exit increments
  `thread_exits_ignored` without producing a record, incident, or failed lease.
- [x] AC3: The child leader exit produces exactly one accepted
  `KernelProcess/ProcessExit` record and one Critical `UserComponentExit`
  incident with `LatchFault` and confidence 100.
- [x] AC4: Evidence and incident fields preserve child PID/TID, boot/epoch,
  hard-fact scalar semantics, zero cycle/transition, and zero loss.
- [x] AC5: Kernel statistics prove one emitted event, one leader exit, one
  ignored worker exit, zero OOM events, and no ring-buffer loss.
- [x] AC6: The observer lease is Healthy before leader exit and Failed after
  the incident, with one incident, the exact attach bit, zero loss, and the
  latched-incident fault code.
- [x] AC7: The exact-schema validator accepts the fixture report and regression
  tests reject malformed schema/types, partial attachment, false worker
  incidents, semantic/count/identity mismatches, loss, and health mismatch.
- [x] AC8: `make test-ebpf-process-exit-runtime` builds unprivileged, elevates
  only execution, fails explicitly without privilege, and validates output.
- [x] AC9: GitHub Actions runs the privileged qualification, uploads
  `build/ebpf_process_exit_qualification.json`, and all focused/full quality
  gates pass.
- [x] AC10: Documentation preserves the observation-only boundary and all
  unqualified lifecycle cause, target-kernel, overhead, and long-run limits.

## Out Of Scope

- OOM injection or qualification, exit code/signal attribution, process
  restart supervision, all-thread termination proof, PID namespace or cgroup
  identity, and cross-process parent/child causal inference.
- ROS 2 controller, recorder, gateway, or agent restart policy and automatic
  process management.
- Direct writes from eBPF or the agent to MLG state, PDOs, CiA 402 controlwords,
  motion permits, or production configuration.
- Production kernel allowlists, probe overhead, WCET, soak, signing,
  systemd/cgroup packaging, deployment rollout, and target hardware HIL.
