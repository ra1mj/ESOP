# Design: eBPF raw-port runtime qualification

## Qualification Boundary

The executable is a privileged host integration fixture. It reuses the exact
production marker symbols, BPF object, Aya loader, attachment API, ring-buffer
decoder, statistics reader, and incident correlator. The injected operation is
a blocking Unix-domain `recv(2)` released by a bounded helper thread; therefore
it proves real syscall-duration observation through the shared marker ABI but
does not prove AF_PACKET or hardware behavior.

Production `LinuxRawPort`, no-std EtherCAT code, simulator semantics, and motion
authority remain unchanged.

## Runtime Flow

1. Build `bpf/build/esop_runtime.bpf.o` with the bundled `bpf/vmlinux.h`.
2. Start the fixture as root and derive `tracked_pid` from its process ID.
3. Load with `enabled_attach_mask = 0` and `required_attach_mask = 0` so unrelated
   tracepoints cannot affect the isolated result.
4. Require `attach_raw_port_probes(current_exe, Some(pid), true)` and assert both
   raw-port bits are present in the runtime and capability snapshot.
5. Publish cycle sequence 1 with WKC risk to the kernel context and agent.
6. Create a Unix stream pair. A helper thread sleeps for the configured delay,
   then writes one byte. The main thread emits RX begin, calls `libc::recv`,
   captures the result/error, and emits RX/frame or RX/error end exactly once.
7. Poll a fixed number of times with a hard elapsed-time deadline until one
   incident is available.
8. Assert incident, evidence, poll, attach, and kernel-stat invariants.
9. Atomically replace the requested output with the JSON report only after all
   checks pass.

## Timing And Correlation

The threshold is 5 ms and the helper delay is 25 ms, leaving margin for hosted
runner scheduling while keeping the test short. Evidence carries cycle sequence
1 from the kernel context, so the correlator uses exact cycle identity rather
than relying on a loose timestamp window. The agent observes the same WKC-risk
cycle before the syscall begins.

The poll budget is bounded to 200 iterations with 5 ms sleep and a 2-second
wall deadline. A missing ring-buffer record is a hard qualification failure.

## Artifact Contract

`build/ebpf_raw_port_qualification.json` is a versioned, exact-schema object.
It records:

- `schema_version`, `status`, threshold and injected delay;
- runtime/required attach masks and poll counts;
- begin/completion/stall/mismatch/lost-event counters;
- incident code/severity/action/confidence and identity fields;
- evidence ID, cycle, operation detail, observed value, threshold, and duration.

The fixture owns semantic assertions; the Python validator independently
re-parses the artifact, rejects bool-as-int and unknown/missing fields, and
checks cross-field invariants. The report uses only fixed strings and numeric
values, avoiding an extra serialization dependency in the runtime crate.

## Build And Privilege Model

`scripts/test-ebpf-raw-port-runtime.sh` performs compilation without privilege.
Only the already-built fixture executes through `sudo -n` when the caller is not
root. A missing passwordless privilege path fails explicitly. GitHub Actions
uses the hosted runner's sudo and uploads the validated report from a dedicated
job.

## Failure And Rollback

Verifier, load, symbol lookup, pair attachment, syscall, polling, decoding,
correlation, statistics, and validation failures all produce a nonzero exit.
The output is written through a temporary file and renamed only after success,
so stale/partial reports cannot masquerade as qualification.

Rollback removes the fixture, validator, Make/CI job, and documentation claim;
the production BPF and raw-port implementation are not modified by this task.

## Remaining Limits

Passing this job does not qualify AF_PACKET sockets, real EtherCAT interfaces,
TX queue backpressure, nonblocking RX under load, driver/NIC/wire/slave timing,
probe overhead, hard realtime WCET, or long-duration operation.
