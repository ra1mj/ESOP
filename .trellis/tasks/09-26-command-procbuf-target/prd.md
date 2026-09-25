# Write admitted commands into ProcBuf

## Goal

Complete the hosted command-target boundary from schema-v1 `MotionCommand` to
the fixed-layout ProcBuf `CommandPage`. A malformed target must be rejected
before command-policy state changes; a valid target must carry the exact
admitted `MotionPermit` identity into a retryable, bounded ProcBuf publication
that the real-time owner can read without Protobuf or transport dependencies.

## Background

- `esop-ipc::payloads` already owns shared Protobuf command decoding and IPC
  envelope checks for Unix datagrams and Zenoh.
- `CommandIngress` already owns source authorization, authority, TTL, replay,
  rate limiting, policy version, and permit generation.
- `CommandPage` already contains fixed joint targets and most permit identity,
  but it lacks `policy_version` and no hosted adapter currently populates or
  publishes it.
- Protobuf `MotionCommand` already carries `requested_mode` and repeated
  `JointTarget` values. The current v1 schema remains unchanged.

## Requirements

1. Add one shared hosted command-target adapter under the existing
   `esop-ipc` `payloads` feature; no real-time crate may depend on IPC,
   Protobuf, Zenoh, Unix sockets, or allocation.
2. Preserve the existing permit-only decode/admit APIs for compatibility while
   adding an explicit strict path that validates command targets before calling
   `CommandIngress`.
3. Interpret `requested_mode` only as the existing CiA 402 cyclic raw modes:
   CSP `8`, CSV `9`, or CST `10`; all other values are rejected.
4. Use zero-based joint indices, matching `RobotState` projection. Require
   `AXES` to fit the 32-bit mask, every target axis to be in range, unique, and
   selected by `axis_mask`, and every selected axis to have exactly one target.
5. Reject non-finite position, velocity, torque, or limit values and reject
   negative `max_velocity` or `max_torque`. Unselected axes are initialized to
   `JointCommand::EMPTY`; IO outputs remain `IoCommand::EMPTY` because v1
   `MotionCommand` has no IO target field.
6. Reject mask/capacity, mode, joint-count, duplicate, missing, out-of-range,
   unselected-axis, numeric-width, robot, schema, boot, source, sequence,
   layout, or authenticated-source failures before `CommandIngress` mutates
   replay floors, rate windows, or audit state.
7. After successful policy admission, build `CommandPage` from the returned
   `MotionPermit`: identity, epoch, sequence, deadline/expiry, axis mask,
   authority, and policy version must remain equal. Set
   `motion_enable_request = 1` only on this admitted path.
8. Bind the prepared/admitted object to the target ProcBuf robot ID, boot ID,
   layout hash, and compile-time capacities so it cannot be published to a
   different buffer instance or layout.
9. Keep ProcBuf publication a distinct retryable step. A failed double-page
   publication must not require re-running `CommandIngress` or losing the
   admitted command object.
10. Add a no-std lifecycle adapter that reconstructs a `MotionPermit` from a
    validated motion-enabled `CommandPage`, avoiding duplicated field mapping
    in real-time consumers.
11. Exercise the complete IPC path over real filesystem Unix datagrams:
    frame receive, structural target validation, policy admission, ProcBuf
    publication, RT readback, permit reconstruction, and lifecycle acceptance.
12. Add shared Zenoh-facing helpers or methods that reuse the same target
    validation and ProcBuf publication types; Zenoh must not gain an
    independent target mapper.
13. Add `policy_version` to `CommandPage` and bump the ProcBuf ABI version so
    the RT owner can reconstruct the complete admitted `MotionPermit` without
    an out-of-band policy assumption. Old ABI headers must be rejected.
14. Update the capability manifest, README, IPC/command-gateway/Zenoh docs,
    software PRD, and implementation plan to mark command-target conversion as
    delivered while retaining shared-memory/RPMsg, cryptographic identity,
    deployment ACL, production latency/WCET, stress, and HIL limitations.

## Constraints

- Do not change the Protobuf v1 wire schema. The ProcBuf fixed record changes
  only to add the missing permit policy version and therefore requires an
  explicit ABI-version and layout-hash transition.
- Target validation is structural and unit-level. Product-specific mechanical
  limits, PDO scaling, setpoint-step validation, mode confirmation, and actual
  drive behavior remain owned by the RT profile/cycle and HIL qualification.
- Successful policy admission does not itself prove ProcBuf publication or
  drive execution; those outcomes remain separately observable.
- The generic `esop-ipc` transport remains usable without the `payloads`
  feature.

## Acceptance Criteria

- [x] A strict shared adapter maps valid CSP, CSV, and CST Protobuf targets into
      fixed ProcBuf arrays with exact permit identity and empty unselected/IO
      slots.
- [x] ProcBuf ABI v5 carries `policy_version`; v4 headers and mismatched layout
      hashes are rejected before attachment or publication.
- [x] Invalid modes, capacities, masks, duplicate/missing/out-of-range targets,
      non-finite values, and negative limits are deterministically rejected.
- [x] Tests prove every structural target failure occurs before ingress audit,
      replay, and rate-limit state changes.
- [x] An admitted command is bound to the originating ProcBuf identity and
      cannot publish into a different robot, boot, layout, or capacity.
- [x] ProcBuf publication can be retried from the same admitted value without
      invoking `CommandIngress` again.
- [x] A real Unix datagram command reaches ProcBuf, is read by the RT side, and
      yields a lifecycle-accepted `MotionPermit` with unchanged target values.
- [x] Zenoh and IPC target paths share one mapper and existing permit-only APIs
      remain source-compatible and green.
- [x] Documentation and capability claims describe the delivered boundary and
      remaining production qualification gaps accurately.
- [x] Focused tests, strict Clippy, `make ci`, and pushed GitHub Actions pass.
