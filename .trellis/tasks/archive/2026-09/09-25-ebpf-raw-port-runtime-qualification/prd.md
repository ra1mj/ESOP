# eBPF raw-port runtime qualification

## Goal

Turn the Linux raw-port stall path from source/build evidence into an
executable privileged qualification that proves the repository's actual CO-RE
object can be verified, loaded, attached to both stable raw-port markers,
driven by a delayed syscall, consumed from the ring buffer, and correlated into
`HostPortStall` on a supported Linux host.

The qualification must produce machine-validated evidence in CI. It remains a
host integration test and must not be presented as AF_PACKET, driver, NIC,
wire, slave, WCET, or production realtime qualification.

## Background

- `BpfRuntime` already loads the CO-RE object, attaches the raw-port begin/end
  pair transactionally, polls the fixed ring buffer, aggregates per-CPU stats,
  and bridges records into `RuntimeAgent`.
- The raw-port marker ABI and BPF state machine are implemented, but current CI
  only builds the BPF object; it does not load or attach it to a running process.
- EBPF-004 and FR-048 explicitly leave target-kernel verifier, real uprobe
  attachment, raw-port stall injection, ring-buffer behavior, and overhead
  qualification open.
- The development host has kernel BTF and can build the object, but unprivileged
  BPF is disabled and local sudo requires interactive credentials. GitHub's
  Ubuntu runner provides noninteractive sudo and is the executable evidence
  environment for this increment.

## Requirements

### R1. Privileged end-to-end qualification executable

- Add a Linux-only qualification executable that loads the repository-built
  BPF object with kernel tracepoints disabled, tracks its own PID, and requires
  successful raw-port begin/end uprobe attachment to its own exact symbols.
- Configure a nonzero raw-port threshold, boot/agent identity, and one
  transport-risk cycle in both the kernel context and `RuntimeAgent`.
- Trigger the real marker pair around an actual blocking `recv(2)` on a local
  Unix socket pair. A bounded helper thread must release the call after a fixed
  delay above the threshold.
- Capture syscall error state before the terminal marker and fail the
  qualification on short/error receive, thread failure, attach failure,
  verifier/load failure, timeout, malformed evidence, or missing incident.
- Bound polling by both iteration count and elapsed wall time; no unbounded
  wait is permitted.

### R2. Evidence and statistics assertions

- Require one `HostPortStall` incident with Error severity,
  `ControlledStop`, confidence 80, the fixture PID/TID, synthetic ifindex 7,
  cycle sequence 1, RX/frame detail, nonzero evidence ID, and strict
  `duration_ns > threshold` with `observed_value == duration_ns`.
- Require the runtime attach mask and required mask to contain the complete
  raw-port pair and no partial pair.
- Require raw-port begins, completions, and stalls to increase; require zero
  raw-port mismatches and no newly reported/lifetime lost events in the isolated
  fixture.
- The executable must only write a `qualified` report after every assertion
  passes. Failure must return nonzero and must not leave a success artifact.

### R3. Machine-readable qualification artifact

- Write a deterministic JSON report under `build/` containing schema version,
  status, threshold/delay, attach masks, poll counters, raw-port kernel stats,
  and the incident/evidence fields needed to audit R2.
- Add a standalone validator that treats JSON as untrusted input, requires the
  exact schema and bounded integer/enum values, rejects booleans as integers,
  and verifies all cross-field invariants.
- Add validator regression tests for the qualified baseline plus missing,
  mistyped, partial-pair, at-threshold, inconsistent-duration, mismatch/loss,
  and wrong-incident cases.

### R4. Reproducible local/CI entry point

- Add one Make target/script that builds the BPF object from the bundled
  `vmlinux.h`, builds the fixture as the unprivileged workspace user, elevates
  only the final executable when passwordless sudo is available, and validates
  the resulting artifact.
- If neither root nor passwordless sudo is available, fail with a precise
  prerequisite message rather than silently skipping or claiming qualification.
- Add a dedicated GitHub Actions job that installs the BPF compiler, runs the
  qualification target, and uploads the validated report. The job is required
  to fail on unsupported verifier/load/attach/ring-buffer behavior.

### R5. Claims and safety boundary

- Update capability/observability documentation and the Trellis quality
  contract with the exact qualified boundary and artifact path.
- Preserve the statement that this fixture validates the shared marker-to-
  incident path using a Unix blocking syscall. Real AF_PACKET timing,
  nonblocking RX behavior under NIC load, driver/NIC/wire/slave attribution,
  overhead/WCET, and long-duration target HIL remain open.
- The qualification path must not write lifecycle state, controlword, motion
  permit, or production configuration.

## Acceptance Criteria

- [x] AC1: The Linux qualification executable exact-links both raw-port marker
  symbols, requires the complete uprobe pair, and drives a bounded delayed
  blocking `recv(2)` under a transport-risk cycle.
- [x] AC2: A successful run observes a strict over-threshold
  `UserEsop/RawPortStall` and produces one `HostPortStall` controlled-stop
  incident with the expected PID/TID/ifindex/cycle/detail/confidence fields.
- [x] AC3: Success requires nonzero begin/completion/stall counters, zero
  mismatch/loss counters, complete attach masks, and a bounded poll loop.
- [x] AC4: The JSON validator accepts the fixture report and regression tests
  reject malformed types, partial attachment, inconsistent timing, loss,
  mismatches, and wrong incident semantics.
- [x] AC5: `make test-ebpf-raw-port-runtime` builds as the normal user, elevates
  only execution, fails explicitly without privilege, and validates the output.
- [x] AC6: GitHub Actions executes the privileged qualification and uploads
  `build/ebpf_raw_port_qualification.json` as evidence.
- [x] AC7: Focused tests, full `make ci`, BPF syntax/CO-RE build, and the remote
  privileged qualification job pass.
- [x] AC8: Documentation states the qualified Unix-socket syscall/marker path
  and keeps AF_PACKET, driver/NIC/wire/slave, overhead/WCET, long-run HIL, and
  production realtime qualification explicitly open.

## Out Of Scope

- Opening an AF_PACKET socket, requiring a physical EtherCAT interface, or
  injecting delay inside a NIC driver.
- Qualification of TX backpressure, nonblocking RX under real packet load,
  queue/NAPI/IRQ/NIC/wire/slave latency, WKC generation, or a complete cycle.
- Performance/WCET acceptance, 30-minute soak, production kernel policy,
  systemd/cgroup packaging, or signing/rollout.
