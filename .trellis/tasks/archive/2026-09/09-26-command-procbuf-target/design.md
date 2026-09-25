# Design: Admitted command targets into ProcBuf

## Ownership and dependency direction

The target mapper remains in the hosted-only `esop-ipc::payloads` module,
which already owns the common Protobuf decoder used by IPC and Zenoh. It may
depend on `esop-proto`, `esop-command-gateway`, `esop-lifecycle-guard`, and
`esop-procbuf`. `esop-procbuf` remains a standalone `no_std` ABI crate and
`esop-lifecycle-guard` keeps its existing optional no-std ProcBuf adapter.

The fixed path is:

```text
MotionCommand bytes / IpcFrame
  -> shared identity and structural target validation
  -> PreparedProcBufCommand<AXES, IO>
  -> CommandIngress::admit
  -> AdmittedProcBufCommand<AXES, IO>
  -> retryable ProcBuf::publish_command
  -> RT ProcBuf::read_command
  -> lifecycle procbuf adapter -> MotionPermit
```

Existing permit-only decoder/admission APIs remain available and delegate to
the same policy-field parser. They do not become target-strict implicitly, so
existing callers are not broken. New IPC and Zenoh target APIs select the
strict path explicitly.

## Prepared command contract

`PreparedProcBufCommand<AXES, IO>` is a fixed-size hosted value containing:

- the expected numeric robot ID, boot ID, and layout hash;
- the policy-bearing `ExternalMotionCommand`;
- validated `ControlMode`;
- `[JointCommand; AXES]` and `[IoCommand; IO]` arrays.

Construction rejects `AXES > 32`, mask bits outside `AXES`, any non-CSP/CSV/CST
mode, and malformed target coverage. Joint indices are zero-based. The
selected mask and target list form a bijection: one target for every selected
axis and none elsewhere. All floating-point values must be finite; maximum
velocity and torque are non-negative. IO stays empty because the v1 message
does not define IO targets.

IPC preparation first applies the existing envelope/payload checks. Zenoh
preparation applies the same textual robot, boot, authenticated source, and
target checks against an identity derived from the destination ProcBuf.

## Admission and publication

Admission consumes a prepared value only after structural validation has
succeeded. The returned `MotionPermit` is the source of all authority fields
in `CommandPage`; the raw Protobuf values are not copied around the policy
result. `motion_enable_request` is set to one only here.

`AdmittedProcBufCommand<AXES, IO>` owns both the permit and completed page. Its
publication method borrows `self`, validates the destination header identity
and layout, then calls `ProcBuf::publish_command`. Since publication does not
consume the object, `NoFreePage` can be retried without replaying policy
admission. The method returns the ProcBuf publication sequence separately from
the command sequence.

Policy acceptance and buffer delivery remain distinct facts. A successful
ingress audit means the command was authorized, while the publication result
means it became visible to the RT reader. Neither proves that the EtherCAT
frame was transmitted or executed by a drive.

## Real-time handoff

The optional `esop-lifecycle-guard::procbuf` adapter gains one field-preserving
conversion from a validated motion-enabled `CommandPage` to `MotionPermit`.
This avoids each RT owner recreating the mapping. It does not accept the
permit, change lifecycle state, interpret targets, or weaken existing guard
checks.

The current command record omits `MotionPermit::policy_version`, so exact RT
reconstruction is impossible without an ABI change. `CommandPage` therefore
gains that field, its reserved bytes are made explicit, and `ABI_VERSION` moves
from 4 to 5. The existing layout hash changes with the version and record size;
v4 headers are rejected before use. No compatibility shim reads an old command
record as a new one.

ProcBuf `well_formed` validation is tightened at the same boundary: enabled
pages require valid identity/permit fields, nonzero policy version, an
in-capacity axis mask, finite targets, and non-negative target limits.

## Errors and compatibility

Typed errors distinguish target capacity, mode, axis coverage, numeric values,
buffer identity/layout, policy admission, and publication. Existing
`CommandDecodeError` and `CommandFrameError` variants remain source-compatible;
new errors are additive.

No Protobuf schema migration is needed. The frozen v1 compatibility fixture
remains unchanged. ProcBuf consumers must be rebuilt for ABI v5 and already
reject stale versions through header validation. Existing permit-only tests
continue to use messages without target fields, while strict target tests
populate the full command contract.

## Verification and rollback

Unit tests cover the target-validation matrix and prove ingress state is
unchanged for structural failures. Integration tests use real Unix datagram
sockets and a ProcBuf instance, then read the page and pass its reconstructed
permit into `LifecycleGuard`. A separate test publishes the same admitted
value after an initial destination mismatch to prove retry does not require a
second policy admission.

The hosted change is feature-gated with the existing payload adapter. Rolling
back the command-record field also requires rolling back the ProcBuf ABI
version as one atomic deployment unit; mixed v4/v5 processes fail attachment
rather than sharing an ambiguous record.
