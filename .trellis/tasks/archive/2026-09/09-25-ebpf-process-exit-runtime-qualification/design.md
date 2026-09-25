# Design: eBPF process-exit runtime qualification

## Architecture and boundary

The qualification uses the production BPF object, loader, decoder, statistics
ABI, correlator, and health projection without adding a test-only kernel path:

```text
root fixture parent
  -> load CO-RE object with ATTACH_PROCESS_EXIT only
  -> spawn same executable in synchronized child mode
  -> update tracked_pid to child TGID
  -> release and join one child worker thread
  -> prove ignored counter + no evidence/incident
  -> release child leader and wait for normal exit
  -> sched_process_exit tracepoint
  -> fixed 96-byte ProcessExit ringbuf record
  -> BpfRuntime decoder -> RuntimeAgent
  -> Critical UserComponentExit + failed host observation
  -> atomic exact-schema JSON
```

The parent remains alive until validation is complete, so its own leader exit
cannot contaminate the report. Only the child PID is tracked when termination
begins.

## Child synchronization protocol

- The normal mode accepts object/report paths. A private `--tracked-child`
  mode performs no BPF loading and uses piped stdin/stdout as a fixed two-step
  protocol.
- The child waits for a worker-release byte, spawns one worker, joins it, then
  writes a worker-exited acknowledgement. This guarantees the kernel exit hook
  has completed before the parent checks statistics and polls the ring buffer.
- The child then waits for a leader-release byte and returns normally. The
  parent verifies the successful exit status before polling the hard fact.
- Every pipe read/write and child wait is error checked. The parent-side
  evidence poll is bounded by both count and wall time.

## Loader and tracking contract

- `RuntimeConfig.enabled_attach_mask` and `required_attach_mask` are both
  exactly `ATTACH_PROCESS_EXIT`. No scheduler, memory, network, IRQ, gateway,
  or raw-port hooks are enabled.
- Initial tracking uses the parent PID to prevent system-wide process-exit
  noise while the child is created. The existing `update_tracking` context-map
  update switches to the child PID before the first release byte.
- Capability readiness and the runtime/required masks must both equal the
  single process-exit bit. Any unavailable tracepoint or load/attach failure is
  fatal rather than degraded or skipped.

## Runtime assertions

After the worker acknowledgement, statistics must report exactly one ignored
thread exit and zero process events/loss. A poll must report zero records and
incidents, and the observer must remain Healthy.

After the child leader exits, the bounded poll must produce exactly one record
and one retained incident. The fixed evidence fields must match the BPF
producer contract (`KernelProcess`, `ProcessExit`, Critical, scalar one,
zero duration/detail/cycle), and PID/TID must both equal the child PID. The
incident must be `UserComponentExit`, Critical, `LatchFault`, confidence 100.
The resulting heartbeat must be Failed with the existing latched-incident code
`0x45422002`, one incident, the exact attach bit, and no event loss.

## Report and validator

The fixture removes stale report/temp paths on startup and writes through a
same-directory temporary file followed by rename only after all assertions.
The JSON schema is flat and exact to keep the standalone Python validator
small and auditable. The validator bounds every numeric field to `u64`, rejects
Python booleans, requires exact key equality, and checks identity/count/health
relationships rather than only individual values.

## Privilege and CI

The shell entry builds clang/Rust artifacts as the invoking user. It runs only
the final fixture under root or passwordless sudo, then validates the report as
the invoking user. A dedicated Ubuntu 24.04 job is authoritative for privileged
execution and uploads the report.

## Compatibility, risk, and rollback

- No BPF, evidence, statistics, ProcBuf, Protobuf, or external ABI change is
  planned; the increment qualifies existing behavior.
- Hosted kernels may disable BPF or the tracepoint. Required attachment fails
  the job instead of weakening the claim.
- The child protocol avoids timing-only sleeps for lifecycle sequencing. Poll
  sleeps remain bounded and are used only to wait for ring-buffer consumption.
- Rollback removes the fixture, validator/tests, runner, Make/CI entries, and
  documentation claims; production runtime behavior is unchanged.

## Qualification boundary

The test proves one controlled non-leader exit is suppressed and one tracked
leader exit traverses the hosted kernel-to-incident-to-health path. The
tracepoint contains neither exit status nor signal and a leader exit does not
prove all threads have terminated. OOM identity, PID namespaces/cgroups,
component restart behavior, production target kernels, overhead, and soak
remain separate evidence.
