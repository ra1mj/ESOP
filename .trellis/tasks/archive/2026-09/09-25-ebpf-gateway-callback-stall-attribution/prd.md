# eBPF gateway callback stall attribution

## Goal

Extend the bounded optional gateway observation path from outbound publish
operations to inbound Zenoh command-subscription and query callbacks. The
runtime must measure the complete synchronous callback invocation without
changing callback semantics or the real-time control path, and must emit the
same fixed `GatewayStall` evidence only for validated over-threshold work.

## Background

- FR-048 and the capability manifest explicitly leave Zenoh query/subscribe
  user-space critical functions open.
- `subscribe_commands` and `serve_queries` currently hand caller callbacks
  directly to Zenoh. `serve_typed_queries` executes decode, provider work,
  reply encoding, and synchronous reply wait inside the `serve_queries`
  callback, so wrapping that outer callback measures the full handler.
- The publish increment already provides a process-global non-zero request
  sequence, a bounded `{TGID, request_id}` LRU map, fixed 96-byte evidence,
  threshold policy, lifecycle correlation, and atomic uprobe-pair attachment.
- Publish and callback markers must remain distinct stable symbols. Reusing a
  Rust closure or `async fn` implementation symbol would not be a stable ABI.

## Requirements

### R1. Stable callback observation ABI

The Zenoh-enabled gateway shall export versioned unmangled C ABI begin/end
markers for callback execution. Marker arguments shall be fixed integers:
non-zero request ID, route kind, and terminal outcome. Callback markers shall
be distinct from publish markers.

### R2. Complete callback lifecycle

Command subscription callbacks and raw/typed query callbacks shall be wrapped
immediately around the user callback invocation. Normal return shall emit one
completed end marker. Unwinding or other guard abandonment shall emit one
abandoned end marker through `Drop`; double completion is forbidden.

### R3. Shared collision-free operation identity

Publish and callback observations shall use one process-global non-zero atomic
request sequence so the shared BPF operation map cannot collide between
operation classes in normal execution.

### R4. Operation-class validation

The BPF operation state shall record whether the begin event is publish or
callback. Publish probes shall accept only State/Event/Diagnostic routes;
callback probes shall accept only Command/Query routes. End probes shall reject
cross-class, route, outcome, PID, request, or policy-epoch mismatches and remove
resolved state before returning.

### R5. Atomic optional callback attachment

The Aya runtime shall expose a callback begin/end attachment API using exact
marker symbols. The pair shall be transactional and idempotent. Optional
unavailability shall return `Ok(false)` without degrading the kernel baseline;
required unavailability shall return an error, and partial attachment shall be
rolled back.

### R6. Thresholded fixed evidence

Callback duration shall use the existing validated non-zero gateway threshold.
Only `duration_ns > threshold` may emit 96-byte
`UserZenoh/GatewayStall` evidence. The event shall preserve request ID, TGID,
honest TID attribution, cycle context, route/outcome detail, and existing
classifier consistency checks.

### R7. Compatibility and isolation

Existing publish probes, kernel tracepoints, fixed map/stat/event ABIs, no-std
gateway builds, callback signatures, typed query error behavior, and live
Zenoh behavior shall remain compatible. Unattached markers shall be fixed
no-op calls and shall not introduce blocking or allocation into callbacks.

### R8. Documentation truthfulness

README, runtime/gateway observability docs, FR-048, capability manifest, and
the executable Trellis quality contract shall state that publish and
query/subscribe callback stalls are implemented while ROS2, recorder,
target-kernel attach/injection, and overhead qualification remain open.

## Acceptance Criteria

- [x] AC1: Exact callback begin/end v1 symbols link from a Zenoh-enabled
  integration test and remain separate from publish marker symbols.
- [x] AC2: One shared allocator produces non-zero unique IDs across publish and
  callback observations; completed and abandoned callbacks emit exactly one
  matching end marker.
- [x] AC3: Command and query registration wrap callback invocation without
  changing public callback signatures; typed query decode/provider/reply work
  remains inside the measured query callback.
- [x] AC4: BPF callback begin/end programs share the bounded 1024-entry map but
  validate callback operation class and Command/Query routes; publish probes
  reject callback routes and cross-class completion emits no evidence.
- [x] AC5: Callback probe attachment is an atomic optional/required pair with
  dedicated capability bits and rollback of the first link on second failure.
- [x] AC6: A valid callback over threshold preserves the 96-byte evidence ABI,
  request/PID/TID attribution, duration/threshold consistency, and fixed
  route/outcome detail; at/below-threshold work emits no stall.
- [x] AC7: Existing publish, decoder, classifier, kernel tracepoint, no-std,
  typed query, and live Zenoh tests remain green.
- [x] AC8: Documentation and capability data remove query/subscribe from the
  open gateway-hook list without claiming ROS2, recorder, real target attach,
  fault injection, or production overhead qualification.
- [x] AC9: `make ci`, `make test-zenoh`, a real Clang BPF object build, and the
  repository GitHub Actions workflow pass before archival.

## Out of Scope

- Callback-specific incident codes or event wire-layout changes.
- Measuring Zenoh's internal dispatch before callback entry or after callback
  return.
- ROS2 control, recorder, Linux RT-port, permit, reconnect, and configuration
  tool markers.
- Target-kernel privileged uprobe qualification, fault injection, soak, and
  production overhead qualification.
