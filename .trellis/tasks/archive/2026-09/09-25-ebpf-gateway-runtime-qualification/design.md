# Design: eBPF gateway runtime qualification

## Qualification Boundary

The fixture reuses the exact production marker symbols, BPF object, Aya loader,
transactional pair attachment, ring-buffer decoder, statistics reader, and
incident correlator. It calls the markers directly around deterministic sleeps,
so the executable proves both gateway operation classes through the shared
observation chain without requiring external network services.

Direct marker calls intentionally do not claim live Zenoh transport behavior.
Production gateway routing, command ingress, lifecycle authority, and realtime
EtherCAT code remain unchanged.

## Runtime Flow

1. Build `bpf/build/esop_runtime.bpf.o` and the Linux fixture.
2. Start the fixture as root and derive tracked PID/TID from the process.
3. Load with kernel tracepoints disabled so unrelated host events cannot affect
   the isolated counters or artifact.
4. Require publish and callback probe pairs on `current_exe` for the fixture PID
   and assert runtime/capability masks equal the four gateway bits only.
5. Publish cycle sequence 1 with WKC risk to the BPF context and agent.
6. Emit Diagnostic publish begin, sleep 25 ms, then success end using request ID
   101. Emit Command callback begin, sleep 25 ms, then completed end using
   request ID 202.
7. Poll at most 200 times, with a 5 ms sleep and 2-second wall deadline, until
   both records are accepted and one retained incident is available.
8. Assert attach, poll, statistics, incident, and both evidence contracts.
9. Atomically replace the requested output with JSON only after all assertions
   pass.

The fixed request IDs make the report deterministic and audit exact propagation
from marker argument through BPF map key to evidence ID. They remain distinct
and nonzero as required by the ABI.

## Incident Merge Contract

The agent correlation window is 1 second. Both delayed operations run on the main
thread and complete within that window with the same PID/TID, code, boot, and
agent epoch. The first evidence creates one incident; the second merges into
the retained incident. The final queue therefore contains one incident with
`count = 2`, `evidence_count = 2`, and two evidence records in operation order,
while `PollReport::incidents_emitted` is two because both records classified.

This checks the existing bounded merge semantics rather than bypassing them
with separate agents or draining between operations.

## Detail And Timing Contract

Gateway detail is `(outcome << 4) | route_kind`. The expected values are:

- publish Diagnostic (2), Success (0): `0x02`;
- callback Command (3), Completed (0): `0x03`.

The threshold is 5 ms and each injected delay is 25 ms. The fixture requires
strict over-threshold duration and exact observed-duration equality for each
record. Evidence IDs must equal 101 and 202 respectively.

## Artifact Contract

`build/ebpf_gateway_qualification.json` is a versioned exact-schema object. It
stores common runtime, attach, poll, statistics, and incident fields plus
prefixed publish/callback request, detail, timing, identity, and count fields.
Using flat fixed fields makes the success contract explicit and avoids adding a
serialization dependency to the runtime crate.

The Rust fixture owns runtime assertions. The Python validator independently
re-parses untrusted JSON, rejects bool-as-int and unknown/missing keys, and
rechecks masks, exact counts, enums, distinct IDs, details, timing, loss, and
mismatch invariants.

## Build And Privilege Model

The shell entry point performs BPF and Rust compilation without privilege. Only
the already-built fixture executes through `sudo -n` when the caller is not
root. Missing passwordless privilege fails explicitly. GitHub Actions runs the
same target and uploads the validated artifact from a dedicated job.

## Failure And Rollback

Verifier, load, symbol lookup, pair attachment, polling, decoding, correlation,
statistics, semantic assertion, and schema validation failures all return
nonzero. The fixture removes stale output before running and uses a temporary
file plus rename after success, preventing partial or stale qualification.

Rollback removes the fixture, validator/tests, Make/CI job, and documentation
claim. The production gateway marker implementation and BPF state machine are
not changed by this task.

## Remaining Limits

Passing this job does not qualify live Zenoh sessions, router/transport queues,
network delay/loss/reconnect, IPC, serialization, authorization, motion permit,
production kernels, probe overhead, WCET, or long-duration operation.
