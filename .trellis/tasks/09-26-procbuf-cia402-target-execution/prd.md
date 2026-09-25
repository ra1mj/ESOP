# Execute ProcBuf targets through CiA 402 PDO

## Goal

Convert admitted ProcBuf v5 CSP/CSV/CST targets through frozen per-axis SI-to-raw scaling and mechanical limits, then submit them through the existing lifecycle-qualified CiA 402 active EtherCAT frame path without partial state mutation.

## Background

- The hosted IPC/Zenoh path now publishes structurally validated SI-valued
  targets and complete permit identity into ProcBuf ABI v5.
- The real-time side can reconstruct and accept the exact `MotionPermit`, but
  the production CiA 402 cycle still requires callers to manually provide raw
  `Cia402Target` and `CyclicLimits` arrays.
- `submit_active_frame` already owns verified Domain input, mode and PDS gates,
  first-enable actual-target matching, setpoint guards, transactional image
  construction, and commit-after-TX semantics.
- Product-specific SI-to-PDO conversion, mechanical limits, and binding of the
  current ProcBuf command to the guard's accepted permit are still missing.

## Requirements

1. Add the ProcBuf-to-CiA-402 command adapter in the real-time dependency
   direction. `esop-lifecycle-guard` may use optional ProcBuf, profile, and
   EtherCAT features; no real-time crate may depend on IPC, Protobuf, Zenoh,
   filesystem APIs, allocation, or `std`.
2. Define one frozen per-axis policy containing finite, nonzero signed scales
   for position, velocity, and torque; a raw position offset; SI mechanical
   position bounds; SI velocity and torque bounds; and a maximum SI position
   step per cycle.
3. Validate every selected axis transactionally before touching setpoint
   guards, process images, frame state, or the port. Invalid policy fields,
   zero cycle period, non-finite values, raw numeric overflow, mechanical
   position violations, and command velocity/torque caps outside product
   policy must return typed errors containing the axis.
4. Interpret ProcBuf positions as radians, velocities as radians per second,
   and torque as newton-metres. Convert active values to signed CiA 402 raw
   units with deterministic nearest-integer rounding; convert non-negative
   guard limits conservatively by rounding down.
5. Preserve axis direction inversion through signed scales and apply the raw
   position offset only to absolute position targets, not to velocity, torque,
   or step/limit magnitudes.
6. The command's `max_velocity` and `max_torque` must not exceed the frozen
   product maxima. CSV/CST targets must stay within both command and product
   caps. CSP position must stay inside mechanical bounds, and its effective
   raw step limit must be the minimum of the frozen per-cycle step and the
   command velocity cap multiplied by the configured cycle period.
7. Require a validated motion-enabled ProcBuf command whose reconstructed
   permit exactly equals the permit currently held by `LifecycleGuard`.
   Reject disabled, expired, malformed, wrong-mode, wrong-axis-mask, stale
   policy/epoch/sequence, or otherwise mismatched command snapshots before
   active output preparation.
8. Require the ProcBuf global requested mode to match every selected axis's
   frozen CiA 402 operating mode. Unselected axes produce no active target and
   may not inherit another axis's command or policy.
9. Produce a fixed-size prepared command containing desired raw targets and
   raw `CyclicLimits`. The prepared value must retain command/permit identity
   so the execution boundary can recheck that it belongs to the current
   lifecycle decision.
10. Integrate the prepared command with the existing active EtherCAT path.
    During PDS handshaking no target is written; on the Switched On to
    Operation Enabled edge the frame must use current verified actual feedback
    as the hold target; only an already Operation Enabled axis may receive the
    desired ProcBuf target.
11. Reuse the existing `submit_active_frame` invariants for Domain freshness,
    WKC, deadline, writable PDO coverage, cross-axis alias checks, setpoint
    validation, frame construction, TX, fallback stop, and guard commit. A
    conversion, build, or TX failure must not advance a setpoint guard or
    publish the desired target as accepted.
12. Preserve all existing public raw-target APIs and tests. The new prepared
    path is additive and must not weaken callers that intentionally provide
    exact raw targets for simulation or low-level integration.
13. Add an end-to-end software test covering ProcBuf publication/readback,
    permit reconstruction and rearm, SI conversion, first-enable actual hold,
    subsequent desired PDO target, simulated drive feedback, and rejection
    without TX/guard mutation.
14. Update README, capability claims, IPC/lifecycle/robotics/PRD documentation,
    and backend guidelines to distinguish the delivered software execution
    path from still-open product configuration generation, real-drive HIL,
    braking/mechanical suitability, WCET, and safety qualification.

## Constraints

- Do not change the Protobuf v1 schema or ProcBuf ABI v5 layout in this task.
- The adapter applies caller-frozen product policy; it does not invent drive
  scales from SII/ESI, vendor objects, or runtime feedback.
- Software conversion and simulated PDO submission are not evidence that a
  physical machine is safe to energize. Real drive firmware, gearing,
  braking, STO, external safety, and HIL qualification remain separate gates.
- All arrays and errors remain fixed-size and allocation-free.

## Acceptance Criteria

- [x] Frozen per-axis policies validate signed SI-to-raw scales, raw offset,
      mechanical bounds, product caps, and per-cycle position-step limits.
- [x] Valid CSP, CSV, and CST ProcBuf commands produce deterministic raw targets
      and conservative raw `CyclicLimits`, including inverted axes and offsets.
- [x] Non-finite/invalid policy, raw overflow, mechanical/cap violations,
      zero cycle period, mode mismatch, axis mismatch, expiry, disabled motion,
      and permit mismatch are rejected with stable axis-aware errors.
- [x] Every preparation failure occurs before setpoint guard, process image,
      frame, port, and lifecycle state mutation.
- [x] The active cycle writes no target during handshake, writes verified
      actual feedback on the enable edge, and writes the desired command only
      after Operation Enabled is confirmed.
- [x] A failed frame build or TX leaves desired-target guards unchanged and the
      existing fail-closed stop fallback remains observable.
- [x] Existing raw-target APIs remain source-compatible and their tests pass.
- [x] A software end-to-end test proves ProcBuf command publication through
      permit acceptance, SI conversion, CiA 402 PDO submission, and simulated
      drive feedback for all three cyclic modes or a representative multi-mode
      matrix.
- [x] Documentation and capability claims remove the generic software-path
      gap without claiming real-drive, mechanical, WCET, or HIL qualification.
- [x] Focused tests, strict Clippy, and `make ci` pass. Pushed GitHub Actions is
      completed during the task commit and archive flow.
