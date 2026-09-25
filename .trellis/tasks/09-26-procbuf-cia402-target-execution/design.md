# Design: ProcBuf command execution through CiA 402 PDO

## Boundary and ownership

The adapter belongs to the optional real-time integration surface of
`esop-lifecycle-guard`. ProcBuf owns the fixed command record, the lifecycle
guard owns permit/state authority, the CiA 402 profile owns raw cyclic target
semantics, and EtherCAT owns process-image and frame submission. The data flow
is:

```text
ProcBuf::read_command
  -> motion_permit_from_command -> LifecycleGuard::request_rearm/renewal
  -> prepare ProcBuf command against current guard permit + frozen axis policy
  -> fixed PreparedCia402Command { desired targets, raw guard limits, identity }
  -> StopCycleContext active branch
  -> resolve handshake/enable-edge/active target from verified Domain feedback
  -> existing submit_active_frame
  -> accepted EtherCAT frame -> guard state commit -> State/quality publication
```

No hosted transport or serialization type crosses this boundary.

## Frozen axis policy

`Cia402AxisCommandPolicy` is a copyable fixed-size product value. It contains:

- signed raw units per radian, radian/second, and newton-metre;
- an `i32` raw position offset;
- minimum and maximum SI position;
- maximum SI velocity and torque;
- maximum SI position step per cycle.

All floating fields must be finite. Scales must be nonzero. Position bounds
must be ordered and all magnitude limits non-negative. Signed scales allow a
product to reverse one axis without changing command semantics. The position
offset participates only in absolute position conversion.

The policy is supplied by frozen generated/product configuration. This task
does not discover or infer it from a drive at runtime.

## Preparation contract

`prepare_cia402_command` accepts a validated ProcBuf command, current
`LifecycleGuard`, configured per-axis modes and policies, cycle period, and
current monotonic time. It revalidates the command record and requires:

- motion enable is requested and the command is unexpired;
- its complete reconstructed permit equals the guard's current permit;
- the command mask fits the fixed axis count and equals the permit mask;
- every selected configured mode equals the global ProcBuf requested mode.

All selected axes are converted into local staged arrays. A failure returns
without publishing a partial prepared value. Unselected targets remain `None`
and their raw limits remain zero.

Targets use nearest-integer rounding after scaling; range checks happen after
rounding and before casts. Non-negative limits use floor so quantization never
widens an allowed magnitude. CSP uses the command position and an effective
position-step limit of:

```text
min(product max position step,
    command max velocity * cycle period seconds)
```

CSV and CST additionally reject desired magnitudes above the command caps.
All command caps must be no greater than product caps.

The resulting `PreparedCia402Command` retains the exact permit and command
sequence/deadline in addition to desired raw targets and `CyclicLimits`.

## Active-cycle integration

Existing raw-target methods retain their exact semantics. The new ProcBuf
entry points pass prepared targets using a distinct desired-target mode:

- `SwitchOnDisabled` / `ReadyToSwitchOn`: no target field is written;
- `SwitchedOn`: read the current verified actual value for the configured mode
  and use it for the enable-operation edge;
- `OperationEnabled`: use the desired target from the prepared command;
- any other drive state remains fail-closed through existing output checks.

A small EtherCAT helper resolves those cycle targets, then delegates to
`submit_active_frame`. This deliberately reuses its complete validation and
transaction contract instead of creating another PDO writer.

`StopCycleContext` gains additive ProcBuf-command entry points for the direct
and scheduled active paths, plus the unified production-service path with
controlled-stop fallback. Internally the motion input records whether targets
are exact raw targets or desired targets requiring enable-edge resolution.

## Failure behavior

Preparation errors are fixed enums with an axis-bearing policy/conversion
variant. They do not call the port or mutate guards. Existing active submission
errors continue to trigger the production cycle's abort-active and stop-frame
fallback behavior. Target resolution, PDO validation, frame build, and TX
remain transactional; guards are copied and committed only after accepted TX.

No successful software outcome claims physical drive execution. It proves
only that the qualified software path placed the expected raw bytes in an
accepted simulated/port frame under current lifecycle and Domain evidence.

## Compatibility and rollback

ProcBuf ABI v5 and Protobuf v1 remain unchanged. Existing callers can continue
using `run_with_motion` and `submit_active_frame` with exact raw values. The new
types and methods are additive behind the existing `procbuf`, `cia402`, and
`ethercat` features.

Rollback removes the additive adapter and wrappers without changing stored or
wire formats. Capability documentation must be rolled back with the code so it
does not claim an unavailable software execution path.
