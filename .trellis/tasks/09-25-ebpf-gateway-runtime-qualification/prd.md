# eBPF gateway runtime qualification

## Goal

Turn the Zenoh gateway publish/callback stall path from source, unit-test, and
CO-RE build evidence into an executable privileged qualification. The test must
prove that the repository's actual BPF object can be verified, loaded, attached
to all four stable gateway marker symbols, driven through both delayed operation
classes, consumed from the ring buffer, and correlated into one auditable
`GatewayStall` incident on a supported Linux host.

This remains a host integration test. It must not be presented as qualification
of a live Zenoh router, network transport, queueing, IPC, serialization, motion
permit, reconnect behavior, production kernel policy, probe overhead, WCET, or
long-duration operation.

## Background

- `esop-zenoh-gateway` exposes stable v1 C ABI begin/end markers for awaited
  publish operations and synchronous command/query callbacks.
- `BpfRuntime` attaches the publish and callback pairs transactionally, polls
  fixed 96-byte records, aggregates per-CPU statistics, and bridges evidence
  into `RuntimeAgent`.
- The BPF program separates publish and callback route ranges, validates
  operation class/outcome/epoch, and encodes route plus outcome in `detail`.
- EBPF-006 and FR-048 currently leave target-kernel symbol attachment, gateway
  delay injection, ring-buffer behavior, and overhead qualification open.
- GitHub's Ubuntu runner provides noninteractive sudo and is the executable
  evidence environment for this increment.

## Requirements

### R1. Privileged end-to-end gateway fixture

- Add a Linux-only qualification executable that loads the repository-built
  BPF object with unrelated tracepoints disabled, tracks its own PID, and
  requires both publish and callback begin/end pairs on its own executable.
- Configure a nonzero gateway threshold, fixed boot/agent identity, and one
  transport-risk cycle in both kernel context and `RuntimeAgent`.
- Exact-link the production marker functions from `esop-zenoh-gateway`; no
  duplicate test symbols or alternate test-only probe path may be introduced.
- Drive one valid Diagnostic publish/success marker lifecycle and one valid
  Command callback/completed lifecycle using distinct nonzero request IDs. Each
  lifecycle must contain a bounded fixed delay strictly above the threshold.
- Bound polling by both iteration count and elapsed wall time. Verifier/load,
  symbol/attach, timeout, malformed/rejected evidence, or missing incident is a
  hard failure.

### R2. Evidence, incident, and statistics assertions

- Require exactly two accepted ring-buffer records and two classified evidence
  emissions, merged into one retained `GatewayStall` incident for the fixture
  PID/TID and cycle sequence 1.
- Require Error severity, `ControlledStop`, confidence 75, incident count two,
  evidence count two, and zero incident/lost-event overflow.
- Require one publish evidence item with Diagnostic/success detail and one
  callback evidence item with Command/completed detail. Their evidence IDs must
  be distinct, nonzero, and equal the fixture request IDs.
- For both records, require `duration_ns > threshold`,
  `observed_value == duration_ns`, count one, fixture PID/TID, and cycle 1.
- Require runtime and required masks to contain exactly all four gateway bits.
  Require two begins, completions, and stalls, zero gateway mismatches, and zero
  newly reported/lifetime lost events in the isolated fixture.
- The executable may publish a `qualified` report only after every assertion
  succeeds. Failure must return nonzero and leave no success artifact.

### R3. Machine-readable qualification artifact

- Write a deterministic exact-schema JSON report under `build/` containing
  schema/status, threshold/delay, attach masks, poll counters, gateway kernel
  statistics, merged incident fields, and separate publish/callback evidence.
- Add a standalone validator that treats JSON as untrusted input, rejects
  booleans as integers, rejects unknown/missing fields, bounds all unsigned
  integers, and verifies all cross-field invariants from R2.
- Add regression tests for the qualified baseline plus missing/unknown/mistyped
  fields, partial attachment, wrong counts/semantics, duplicate request IDs,
  at-threshold or inconsistent timing, and mismatch/loss cases.

### R4. Reproducible local and CI entry point

- Add one Make target/script that builds the BPF object and Rust fixture as the
  workspace user, elevates only the final executable when passwordless sudo is
  available, and validates the resulting artifact.
- If neither root nor passwordless sudo is available, fail with a precise
  prerequisite message; never silently skip or claim qualification.
- Add a dedicated GitHub Actions job that installs the BPF compiler, runs the
  qualification target, and uploads the validated report. Unsupported
  verifier/load/attach/ring-buffer behavior must fail the job.
- Include validator regression tests in the ordinary `make ci` quality gate.

### R5. Claims and safety boundary

- Update README, FR-048, the runtime observability design, gateway design,
  capability/known-limit documentation where applicable, and the Trellis
  quality contract with the exact qualified boundary and artifact path.
- State that direct marker delay proves the shared marker-to-incident chain and
  both operation classes, not a live Zenoh router or transport-internal cause.
- Keep ROS 2, recorder, gateway IPC/serialization/permit/reconnect, production
  target kernels, performance/WCET, and long-duration HIL explicitly open.
- The qualification path must not write lifecycle state, controlword, motion
  permit, command page, or production configuration.

## Acceptance Criteria

- [ ] AC1: The Linux fixture exact-links all four production gateway marker
  symbols, requires both complete uprobe pairs, and drives delayed publish and
  callback lifecycles with distinct nonzero request IDs.
- [ ] AC2: A successful run observes exactly two strict over-threshold
  `UserZenoh/GatewayStall` records and merges them into one expected
  controlled-stop incident with count/evidence count two.
- [ ] AC3: The retained evidence proves Diagnostic/success publish and
  Command/completed callback details, request IDs, PID/TID, cycle, threshold,
  duration, and observed-value invariants.
- [ ] AC4: Success requires exactly four gateway attach bits, two
  begin/completion/stall counters, zero mismatch/loss, and bounded polling.
- [ ] AC5: The exact-schema validator accepts the fixture report and regression
  tests reject malformed types/schema, partial attachment, wrong route/outcome,
  duplicate IDs, inconsistent timing, loss, and mismatches.
- [ ] AC6: `make test-ebpf-gateway-runtime` builds unprivileged, elevates only
  execution, fails explicitly without privilege, and validates the output.
- [ ] AC7: GitHub Actions executes the privileged qualification and uploads
  `build/ebpf_gateway_qualification.json` as evidence.
- [ ] AC8: Focused Rust/Python tests, full `make ci`, BPF syntax/CO-RE build,
  Zenoh integration, and the remote privileged qualification job pass.
- [ ] AC9: Documentation states the qualified direct-marker path and preserves
  all live transport, production kernel, overhead/WCET, and long-run limits.

## Out Of Scope

- Starting `zenohd`, opening a live Zenoh session, or injecting delay inside
  Zenoh transport, router, queue, serialization, IPC, authorization, or retry.
- Qualifying network loss/reconnect, command/query business logic, callback
  provider correctness, or motion/lifecycle behavior.
- Performance acceptance, probe overhead, WCET, soak, production kernel
  allowlists, systemd/cgroup packaging, signing, or deployment rollout.
