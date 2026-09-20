# Quality Guidelines

The canonical local quality gate is `make ci`. It runs formatting and diff
checks, workspace type checking, all-feature tests, Clippy with warnings
denied, release compilation, the core `aarch64-unknown-none` check, and the
BPF C syntax check. `make bpf` is the additional CO-RE object build and needs
clang, bpftool, kernel BTF, and a Linux BPF-capable host.

## Forbidden Patterns

- Do not add heap allocation, blocking locks, unbounded loops, sleeping, or
  text logging to the activated EtherCAT cycle path.
- Do not use `volatile` as a substitute for synchronization or DMA ownership.
- Do not expose Aya or other Linux-only dependencies from the `no_std` core.
- Do not bypass generation, address, size, type, or working-counter checks when
  consuming a received datagram.
- Do not release a DMA-owned descriptor before the platform reports completion.

## Required Patterns

- Use fixed-capacity arrays, caller-owned buffers, and explicit error enums in
  real-time code. `DmaDescriptorRing` and `RxIndexTable` are the reference
  implementations.
- Make ownership transitions and cache maintenance visible at DMA boundaries.
- Keep frame plans immutable during the active cycle and bind RX expectations
  to the same frame or descriptor generation.
- Admit a received frame only when the complete frame fits the remaining RX
  byte budget and, for polled ports, the poll returned before the cycle's RX
  deadline. Rejected frames must not reach the datagram consumer; record a
  budget diagnostic even when the remaining budget is nonzero.
- Bound poll attempts independently from admitted frames so a malformed-frame
  flood terminates even when the port clock does not advance.
- Add a regression test for every lifecycle or validation change. Prefer
  public integration tests when a change crosses crates.
- Preserve the existing `EthercatPort` copy-compatible path while adding a
  separate DMA trait for zero-copy platform adapters.
- Keep ProcBuf as a standalone `no_std` ABI layer. Use its header/layout hash
  to reject robot/boot/capacity mismatches, and use the double-page ownership
  protocol rather than an unchecked sequence-only copy.
- Bump the ProcBuf ABI version whenever the fixed record layout or field
  semantics change. Keep lifecycle projection in the optional `no_std` guard
  adapter; verify guard -> ProcBuf -> external schema with a public test, and
  reject old headers rather than interpreting them as the new layout.
- Preserve raw cyclic observations separately from debounced lifecycle gates.
  A quality projection must carry known and good bits, match the State sequence,
  and leave incomplete or stale quality absent in the external message.
- In the active guard, a required gate's observed bad value stops motion in
  that cycle even if its exit debounce still marks it qualified. Count at most
  one good observation per cycle; rearm needs a fresh enter-good window.
- Maintenance entry/exit and a latched fault invalidate old gate qualification.
  Preserve per-gate cycle watermarks so same-cycle or replayed evidence cannot
  restore readiness. Fault clearing requires a fresh stable window; clearing
  revokes permits again so rearm needs a post-recovery permit epoch. Maintenance
  must request Disable explicitly rather than rely on Hold's consumer meaning;
  exiting maintenance from Active/Stopping must retain Disable until stop
  acknowledgment, and maintenance toggles must never clear a latched fault.
  The legacy scalar snapshot stop_action is only a default/summary; match
  actual per-axis issued actions to the CiA 402 controlwords instead.
- Revoke the motion permit in the same cycle an active guard enters Stopping.
  Configure the frozen RT `GuardPolicy.allowed_axis_mask` explicitly (the
  conservative default denies all axes), and reject any permit outside it
  even if the ingress gateway accepted the command. Active renewals must keep
  the exact armed axis set; audit invalid renewals without replacing the
  existing permit or advancing its replay cursor. Change axes only after a
  verified stop and explicit rearm.
  Bind stop confirmation to the original armed axis mask and to a complete,
  fresh input sample after a stop action was issued; do not infer stationary
  motion from a disabled CiA 402 state alone. A missing velocity sample or
  unknown drive state cannot confirm a stop. Preserve the first blocker while
  latching an unconfirmed stop at the configured timeout, even across a
  maintenance toggle. Simulator feedback is not HIL qualification evidence.
- `cycle_axes` prepares a stop decision but must not claim it was issued.
  Keep the decision borrowed while building the next PDO; only after the port
  accepts that frame call `mark_stop_transmitted`. A failed TX leaves the
  issuance cycle absent, and ProcBuf `issued_action` stays zero until success.
  Stop confirmation still requires a fresh complete receive after that TX.
- For CiA 402 stop TX, use `submit_stopping_frame` with all mapped axes, a
  caller-verified safe image for other outputs, and a frozen Domain/FramePlan.
  Check that each stop Controlword and mode field is fully covered by a
  writable, matching Domain segment and that axes cannot alias one another's
  output bits. Reject later writable datagrams that overwrite a stop field
  from a different process-image offset. Build errors must release unarmed
  frame slots; TX errors must not mark issuance. A successful submission is
  not drive-execution proof.
- For non-motion cycles, use `submit_inhibited_frame` with the same frozen
  writable Domain/FramePlan and safe image. It must reject Active decisions
  and unsafe axis outputs, and it must never record stop issuance. On a stop
  timeout, publish the fault-latched State before ordered transition/axis
  timeout events even when the inhibited TX fails.
- For the optional single-Domain stop/inhibited branches, use `StopCycleContext` only
  after the real master cycle has completed Domain RX. Match the report to
  both the master cycle and State sequence, and verify the frozen allowed
  axis mask fits the bank, before mutating gates; project RX quality once,
  decide with one guard borrow, submit the next stop or Disable frame,
  stage per-axis evidence even when TX fails, acknowledge only earlier issued
  actions with current verified feedback, then publish State before emitting
  events. A new transition uses this invocation's monotonic timestamp; an
  older transition needs its recorded timestamp from the caller. `NotStopping`
  now denotes Active motion only; the caller still owns its normal output and
  State/event publication. The caller also owns final deadline facts, non-CiA 402 output
  safety, other Domains, and the full cycle scheduler; this branch is not a
  complete production owner or HIL qualification.
- Freeze per-axis stop selections in `AxisStopPolicy` at guard construction.
  Assemble all axis outputs from one borrowed `cycle_axes` decision, use the
  originally armed mask for stop requests, and force every other axis to
  inhibit. Maintenance overrides every armed axis to Disable. The CiA 402
  adapter implements QuickStop/Disable; until a validated controlled target
  generator exists, Hold/RampToZero must fail closed as Disable. ProcBuf v4
  carries per-axis requested and issued actions plus fresh, quality-checked
  feedback proof bits. Bind every sample to the guard decision and State
  sequence, require observed stop feedback strictly after the first stop
  issuance cycle, stage the fixed array before publication, and never infer drive
  execution from a sent controlword. The scalar `stop_action` remains only a
  legacy default/summary; old ABI attachments must fail validation.
- On stop timeout, snapshot the original armed axis mask and the selected
  per-axis stop actions before clearing motion authority. Publish each axis's
  Disable escalation as a bounded ProcBuf event with the full fault code,
  transition sequence, and caller-supplied monotonic timestamp. Failed writes
  must leave only unwritten axes pending for retry; event-ring loss is
  observable. Never treat an event or issued Disable as stationary feedback,
  and do not carry previous-cycle axis stop proof into a fault-latched page.
- Derive CiA 402 stop confirmation from the just-finished verified Domain
  image, never the retained last-good image after a missed receive. Require
  matching receive/decision cycles, complete nonzero WKC, no RX errors or
  budget exhaustion, and fresh validated PDO reads for every armed axis.
  A missing velocity sample is not stationary proof. Before reusing a fixed
  EtherCAT datagram index after a missing response, reap expired expectations
  with the port's monotonic time; unexpired indices must remain armed.
- Publish lifecycle transitions before timeout escalation events through the
  boot-bound, fixed-size event cursor. Advance the cursor only after a ring
  write succeeds, explicitly report overwritten transition history before
  acknowledging the skip, and treat event timestamps as emission time when
  retrying older transitions. `aux` lifecycle states use MLG/ProcBuf raw
  discriminants, not Protobuf enum values.
- A required hard-class gate (platform, configuration, topology, drive, cycle
  budget, or external safety) entering bad or unavailable state while active
  must stage a fault through Stopping and verified stop acknowledgment before
  FaultLatched. Keep controlled-stop gates distinct, preserve the first
  blocker separately from the final latched reason, and treat stale hard-gate
  observations as faults even without an explicit false report.
- Sample EtherCAT-backed MLG facts after finishing all due Domains: require
  this cycle's complete nonzero WKC and fresh DC completion, not a retained
  Domain image or the DC monitor's previously locked state. The cycle owner
  must explicitly supply the other safety facts and final deadline result;
  the simulator's synthetic facts are not device qualification evidence.
- For ProcBuf diagnostics, keep Domain slot order and the due mask aligned with
  the frozen schedule. Project every configured Domain's WKC and age, but only
  qualify scheduled Domains as current; do not overwrite AL, command, or
  fault facts owned by other producers.
- For multi-rate MLG projection, bind each Domain quality snapshot to its
  frozen schedule ID and derive the due tick from master cycle 1 = schedule
  tick 0. An idle tick may reuse only the last successfully scheduled sample
  within its period; current-cycle RX/DC evidence remains mandatory.
- Build slave-to-slave copy plans from an active Domain registry, not ad hoc
  offsets. Bind the source TxPDO, target RxPDO, and target quality RxPDO to
  verified datagram coverage. At the target's scheduled send cycle, accept a
  source sample at most one cycle old; otherwise write the product-specified
  fallback and invalid quality byte. Runtime binding errors leave output
  untouched and require suppressing that target frame.
- Treat `motion_permit_current` as permit freshness, not motion authorization:
  a blocked gate revokes the permit when the guard enters `Stopping`. Only the
  guard's cycle action may authorize CiA 402 enable.
- CiA 402 mode confirmation is not sufficient for motion: cyclic output must
  also require `OperationEnabled`, MLG permission, and a seeded/limited first
  setpoint.
- Keep the CiA 402 EtherCAT adapter behind the profile `ethercat` feature and
  reuse `esop_ethercat_core::PdoEntry` for bit access. Validate standard object
  identity, direction, width, signedness, and overlap before activation; keep
  control/mode handshaking separate from cyclic target writes.
- A typed CiA 402 cyclic write must require all four motion gates
  (lifecycle permit, confirmed mode, Operation Enabled, and valid setpoint) and
  an Operation Enabled Controlword. Preflight every destination field before
  mutating the caller-owned image so rejected writes are atomic.
- Device lifecycle transitions must remain explicit and bounded. A faulted
  device cannot jump directly to `Active` or `Cyclic`; recovery must create a
  new generation and pass configuration phases again.
- Domain/PDO registration must finish before activation. Keep Domain-local PDO
  offsets and datagram payload offsets deterministic, reject process-image and
  logical-address overlap plus global datagram-index reuse, and lock the
  registry after the schedule is activated.
- SII-derived Domain registration must validate the candidate's FMMU/SyncManager
  mapping and Rx/Tx-to-unified-image translation before publishing any
  Domain/PDO entries. Apply the registry update transactionally so insufficient
  image/capacity or a logical-base mismatch cannot leave a partial
  configuration.
- Build multi-frame plans in fixed-capacity temporary storage and publish them
  only after every Domain datagram and the schedule validate. Split only on
  per-frame capacity or encoded MTU; reject a single datagram that cannot fit.
- Auto-bind SII segments only when their logical and physical offsets and bit
  length are byte-aligned. Bit-packed segments require an explicit aggregate
  datagram so byte-oriented Domain staging cannot overwrite neighboring bits.
- Keep host-generated performance reports explicitly unqualified. Accept a
  `passed` scenario only with complete cycle/max-accumulator coverage, matching
  Q1-Q4 load and thresholds, zero fault-free errors, and trace/topology hashes;
  a structurally valid JSON report is not proof of real hardware qualification.

## Testing Requirements

Run `make ci` before handing off a change. Run `make test-hil` when modifying
the Linux port or simulator. New DMA behavior must cover ownership, cache
ordering, stale handles, invalid lengths, error rollback, and at least one
end-to-end simulator path. New eBPF ABI fields must have fixed-size decode and
invalid-discriminant tests. New CiA 402 PDO fields must have public API tests
for all supported modes and a failure-path test proving the output image is
unchanged.

## Code Review Checklist

- Is the worst-case loop bounded by a static capacity or explicit budget?
- Are errors propagated without silently publishing partial process data?
- Are descriptor generation and cache/barrier ordering correct?
- Does the test exercise the public API and the failure path?
- Were `make ci` and the relevant cross-target checks run?
