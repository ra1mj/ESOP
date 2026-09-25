# Connect ProcBuf payloads to hosted IPC

## Goal

Connect the fixed-layout ProcBuf ABI to the hosted Unix Datagram IPC transport through one bounded Protobuf adapter. The adapter must project state and events, decode motion-command frames, enforce agreement between the IPC envelope and Protobuf payload, and be reused by the Zenoh gateway so the two host transports cannot drift.

## Requirements

1. Add one shared host-domain payload module under `esop-ipc`; real-time crates must not depend on it.
2. Project valid `StateSnapshot` and `ProcBufEvent` records into schema-v1 Protobuf payloads carried by `IpcFrame` values.
3. Preserve the existing ProcBuf projector contracts: header validation, single-reader ownership, state replay rejection, lifecycle/stop/quality consistency checks, and finite joint values.
4. Populate IPC envelope identity from validated sources: numeric robot ID and layout hash from the ProcBuf header, configured source ID, ProcBuf boot ID, record sequence, monotonic timestamp, and a documented quality summary.
5. Keep payloads bounded by `esop_ipc::MAX_PAYLOAD_BYTES`; raw Rust or `repr(C)` structures must never cross the IPC transport.
6. Decode `MotionCommand` only from `MessageKind::Command` frames and validate schema, robot identity, boot ID, source ID, sequence, layout hash, and authority width before constructing `ExternalMotionCommand`.
7. When an authenticated source ID is supplied, require it to match both the IPC envelope and Protobuf command source.
8. Keep authorization, TTL, replay, rate-limit, permit-epoch, and policy-version decisions inside `CommandIngress`; the adapter may submit a validated command but must not reproduce policy logic.
9. Reject malformed or inconsistent frames before invoking `CommandIngress`, so pre-policy failures do not mutate ingress replay floors, counters, or audit state.
10. Refactor the Zenoh gateway to call the shared projector and command decoder while preserving its public behavior and compatibility tests.
11. Exercise the adapter over real filesystem Unix datagram sockets for state, event, and command flows.
12. Update the product plan, capability manifest, and IPC/protobuf engineering documentation with the delivered boundary and remaining command-target gap.

## Constraints

- The generic IPC frame and endpoint APIs remain usable without ProcBuf/Protobuf dependencies.
- The payload adapter is a hosted feature and may allocate only after explicit encoded-size validation.
- IPC transport integrity and peer monitoring are diagnostic/admission mechanisms, not SIL claims or cryptographic authentication.
- Motion target conversion into `CommandPage` remains out of scope; this increment ends at a `MotionPermit` returned by the existing ingress policy.
- Shared-memory, RPMsg, ROS 2, HIL, and WCET work remain separate deliverables.

## Acceptance Criteria

- [x] `esop-ipc` exposes a feature-gated shared payload adapter with typed configuration and typed errors.
- [x] A ProcBuf state snapshot becomes a valid `RobotState` IPC frame with matching robot, boot, sequence, timestamp, layout, schema, and quality metadata.
- [x] A ProcBuf event becomes a valid `DiagnosticEvent` IPC frame with matching boot, sequence, timestamp, layout, and source policy.
- [x] A valid command IPC frame decodes to `ExternalMotionCommand` and can be admitted by `CommandIngress` into a `MotionPermit`.
- [x] Envelope/payload identity mismatches, wrong message kinds, invalid schema versions, oversized payloads, non-finite state, and stale state are rejected deterministically.
- [x] Tests prove an envelope/payload mismatch is rejected before ingress audit or replay state changes.
- [x] State, event, and command round trips pass over real nonblocking Unix datagram endpoints.
- [x] The Zenoh gateway has no independent ProcBuf projection or MotionCommand field-mapping implementation.
- [x] Existing Zenoh and ProcBuf behavior tests remain green.
- [ ] `make ci` and the pushed GitHub Actions workflow pass.

## Notes

- This is the next dependency-chain increment after the generic Unix Datagram IPC transport.
- A fixed textual robot ID is still needed for Protobuf, while the numeric robot ID remains the IPC/ProcBuf identity key.
