# eBPF gateway stall attribution

## Goal

Add a bounded, optional eBPF observation path that attributes excessive Zenoh
gateway publish latency to a concrete process and request without changing the
real-time control path. The resulting evidence must let the lifecycle safety
classifier distinguish a measured gateway stall from an uncorrelated or
malformed user-space observation.

## Background

- `docs/esop-software-prd.md` FR-048 leaves user-space critical-function stall
  attribution as an open runtime-observability increment.
- `docs/esop-ebpf-runtime-observability.md` requires stable user-space symbols
  or explicit trace hooks, fixed identifiers, bounded state, and graceful
  `USER_PROBE_UNAVAILABLE` degradation.
- `RuntimeEvidence` already reserves `EvidenceDomain::UserZenoh`,
  `EvidenceKind::GatewayStall`, and `IncidentCode::GatewayStall` while keeping a
  fixed 96-byte ABI.
- Rust async function entry/return probes do not measure the lifetime of the
  awaited operation, so the gateway must expose an explicit versioned
  begin/end observation contract around the publish future.
- eBPF observation is diagnostic only. It must never write MLG outputs,
  controlwords, watchdog permits, or any other control-plane state.

## Requirements

### R1. Stable publish observation contract

The Zenoh gateway shall expose versioned, unmangled C ABI begin/end markers for
the complete asynchronous publish operation. Each operation shall use a
non-zero 64-bit request ID and fixed numeric route/outcome values; no dynamic
string parsing is allowed.

### R2. Complete async lifecycle

The begin marker shall run before the awaited publish starts. Exactly one end
marker shall run for success, transport failure, or future cancellation so a
cancelled future cannot leave stale operation state indefinitely.

### R3. Optional atomic probe attachment

The runtime shall attach the begin/end uprobes as one logical pair to an
explicit gateway executable or library target. A partial attach must be rolled
back. Optional symbol/program absence shall be reported through capability
state without preventing the kernel tracepoint baseline from running; required
mode shall fail closed.

### R4. Bounded operation tracking

The BPF program shall track at most 1024 in-flight gateway operations in an LRU
map keyed by process and request ID. Tracking shall honor the configured target
PID, reject invalid IDs or route values, and discard state across policy-epoch
changes.

### R5. Thresholded evidence

The end probe shall emit `GatewayStall` evidence only when measured duration is
strictly greater than a validated non-zero threshold. The event shall carry the
request ID as `evidence_id`, duration and threshold, process attribution,
honest thread attribution, current cycle context, and fixed route/outcome
detail encoding while preserving the 96-byte evidence ABI.

### R6. Runtime configuration and statistics

Rust and C representations of gateway configuration and runtime statistics
shall remain append-only and size-checked. Invalid threshold updates shall not
mutate live policy. Gateway probe begins, completions, stalls, and mismatches
shall be observable with saturating host-side aggregation.

### R7. Lifecycle classification

The agent shall classify a gateway stall only when the evidence is correlated
to a transport-risk cycle, the threshold is non-zero, measured duration is
strictly over threshold, and `observed_value` is internally consistent with
`duration_ns`. Below-threshold, malformed, or healthy-cycle evidence shall not
produce a gateway-stall incident.

### R8. Compatibility and isolation

Existing kernel tracepoint attachments, fixed evidence decoding, no-std gateway
builds, and deployments that do not enable Zenoh or user probes shall continue
to work. Missing optional probes shall not alter RT execution or event schemas.

### R9. Documentation and capability truthfulness

The runtime observability design, gateway documentation, software PRD, README,
and capability manifest shall describe the implemented publish scope and the
remaining ROS2, recorder, target-kernel attach/injection, and overhead gaps.

## Acceptance Criteria

- [x] AC1: The Zenoh-enabled gateway exports callable versioned begin/end C ABI
  publish markers, and an integration test links against their exact symbols.
- [x] AC2: Concurrent publishes receive non-zero request IDs; success, failure,
  and cancellation paths execute one matching end marker.
- [x] AC3: The runtime exposes an atomic begin/end uprobe-pair attachment API;
  optional unavailability returns a non-fatal result, required unavailability
  returns an error, and partial attachment is detached.
- [x] AC4: The BPF object contains a 1024-entry LRU operation map keyed by TGID
  and request ID, validates target PID/input fields, and removes state on
  completion, mismatch, or policy-epoch change.
- [x] AC5: A valid over-threshold completion decodes as 96-byte
  `UserZenoh/GatewayStall` evidence with request ID, duration, threshold,
  route/outcome detail, PID, and honest TID attribution; at/below-threshold
  completion emits no stall.
- [x] AC6: Gateway configuration and statistics have matching Rust/C size
  assertions, reject invalid threshold updates atomically, and aggregate
  counters without overflow.
- [x] AC7: Classifier tests accept only internally consistent over-threshold
  evidence in a transport-risk cycle and reject malformed, at-threshold,
  below-threshold, and healthy-cycle cases.
- [x] AC8: Existing evidence decoder, kernel tracepoint, no-std, and Zenoh
  gateway tests remain green when gateway probes are not attached.
- [x] AC9: Documentation and `capability_manifest.json` state that this
  increment covers gateway publish markers only and retain the remaining
  observability limitations.
- [x] AC10: `make ci`, the live Zenoh test target when available, the BPF object
  build, and the repository GitHub Actions workflow pass before archival.

## Out of Scope

- ROS2 control, recorder, query-handler, and subscription callback stall
  probes.
- Automatic symbol discovery across arbitrary stripped binaries.
- Target-kernel verifier/attach qualification, fault injection, production
  overhead qualification, and long-running soak validation.
- Any eBPF-mediated control, restart, permit, or MLG decision.
