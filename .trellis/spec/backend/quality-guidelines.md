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
- For the optional lifecycle branches, use `StopCycleContext` only after the
  real master cycle has completed RX for the motion Domain. Match the report to
  both the master cycle and State sequence, and verify the frozen allowed
  axis mask fits the bank, before mutating gates; project RX quality once,
  decide with one guard borrow, submit the next stop, Disable, or active frame,
  stage per-axis evidence even when TX fails, acknowledge only earlier issued
  actions with current verified feedback, then publish State before emitting
  events. A new transition uses this invocation's monotonic timestamp; an
  older transition needs its recorded timestamp from the caller. The stop-only
  `run` rejects Active; `run_with_motion` binds each caller-owned target guard
  to the boot ID and Active transition sequence. On a new activation, discard
  the old setpoint seed; only a matching Switched On actual-feedback sample may
  seed it at the enable-operation edge. Permit renewal during the same Active
  transition must not reset the seed. Require current verified RX and matching
  Statusword, confirmed mode and Operation Enabled for moving targets. The
  Switched On -> Operation Enabled edge must also carry a target equal to the
  current actual feedback; earlier PDS handshake steps carry no target. Never
  send Enable Operation with a stale target from the safe image. Reject target
  and Controlword aliases even
  against an unpermitted axis. Commit setpoint guards only after successful TX.
  On an active validation/build/TX error, preserve the initial error, abort
  motion and attempt a stop PDO in the same cycle; even if that stop TX also
  fails, publish zero issued evidence, the Stopping State, and ordered events.
  For multi-Domain cycles, require the frozen schedule's exact Domain count,
  ID order, and a motion Domain scheduled every tick whose snapshot matches
  the actual Domain. Project caller-owned snapshots for all other Domains,
  including missing due receives; never silently use the one-Domain entry
  with a multi-Domain State page. A configuration error must fail before TX
  or gate mutation, and the caller must handle that error as a safety failure.
  The caller still owns final deadline facts, verified quality snapshots for
  other Domains, their output scheduling, non-CiA 402 output safety, and the
  full cycle scheduler; this branch is not a complete production owner or
  HIL qualification.
- Deadline-checked `StopCycleContext` entries take the current cycle's
  absolute deadline, distinct from the next TX frame's RX deadline. Use the
  port's monotonic clock for permit freshness and check it both before TX and
  after the last attempted TX, before State publication. A pre-TX miss must
  suppress active output. If an accepted active TX finishes late, revoke the
  permit and publish a blocked budget gate, Stopping, and an unissued stop
  request in that same cycle. The old TX may still own its RX index: report
  that active TX truthfully and send the stop once the next cycle can reuse
  the index. Also sample after State/event publication attempts. If the
  publication crossed the deadline, revoke the permit, block the budget,
  stage an unissued stop request for any accepted active TX, and try a
  corrected State before emitting newly pending events. Report both initial
  and corrective publication errors explicitly; an already published Active
  snapshot or event cannot be retracted, even when correction succeeds. The
  full-cycle owner must handle corrective failure and include remaining TX,
  scheduling, and hardware timing in its own qualification.
- Freeze per-axis stop selections in `AxisStopPolicy` at guard construction.
  Assemble all axis outputs from one borrowed `cycle_axes` decision, use the
  originally armed mask for stop requests, and force every other axis to
  inhibit. Maintenance overrides every armed axis to Disable. The CiA 402
  adapter's default path implements QuickStop/Disable and continues to degrade
  Hold/RampToZero to Disable. The opt-in controlled path may use
  `ControlledStopPlanner` only with product-frozen raw-unit limits and a
  current, completed Domain: Hold is CSP-only and locks the first verified
  actual position; RampToZero is CSV/CST-only and approaches zero from current
  verified velocity/torque. Freeze action, mode, limits, and guard transition
  sequence for the entire stop. Preview planner state transactionally, commit
  only after the complete frame is accepted by the port, and arm the next
  Domain receive only after that submission. A missing mode/feedback/target
  mapping, stale cycle, changed limits, build failure, or TX failure must leave
  planners and stop-issued evidence unchanged. Production-cycle integration
  must own one `ControlledStopCycleState` with frozen limits. The first
  controlled validation, build, or TX failure in an MLG transition sequence
  must reset the planners and latch every later cycle in that sequence to the
  default Disable/QuickStop path; a new transition sequence clears that latch.
  An active-frame failure that creates the stop sequence must enter the same
  latched fallback path. ProcBuf v4 carries per-axis requested and issued
  actions plus fresh, quality-checked feedback proof bits. Use the controlled
  evidence projector only for an accepted controlled frame so an enabled
  Hold/Ramp target is recorded as the policy action and its terminal Disable
  is recorded as Disable. Bind
  every sample to the guard decision and State sequence, require observed stop
  feedback strictly after the first stop issuance cycle, stage the fixed array
  before publication, and never infer drive execution from a sent controlword.
  The scalar `stop_action` remains only a legacy default/summary; old ABI
  attachments must fail validation.
- Controlled-stop contract: **Scope**: opt-in CiA 402 stopping only; default
  cycle APIs remain fail-closed, while the explicit controlled production
  entries require a caller-owned `ControlledStopCycleState`.
  **Inputs**: one borrowed stop decision, matching current `CycleReport`,
  completed Domain inputs, frozen maps/modes/limits, safe process image, and
  writable frame plan. **Outputs**: one accepted frame report with per-axis
  outputs and optional controlled commands, followed by matching ProcBuf
  evidence. **Invariants**: no allocation, no replay, no cross-axis alias,
  fixed action/mode/limits/transition sequence, and no normal motion permit.
  **Failure**: reject before TX where possible; build/TX failure commits neither
  planner state nor issuance evidence, latches the sequence to the default stop,
  and exposes both the controlled error and fallback status in cycle/release
  evidence. A fallback frame may still fail and must be reported independently.
  **Boundaries**: raw-unit limits, mechanical suitability, braking, STO, WCET,
  and real drive behavior belong to product qualification. **Tests**: cover
  Hold capture, CSV/CST ramp bounds, minimum integer values, changed policy,
  missing feedback, failed TX retry, PDO bytes, terminal Disable, and evidence.
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
- Bind heterogeneous, configured RX Domains to the frozen schedule through
  `ScheduledDomainBank` at activation. Reject duplicate datagram indices or
  ID order changes before starting any receive. Start and finish every due
  Domain exactly once per master cycle, including missing or failed receives;
  non-due data must never stage into that Domain. The bank's fixed index map
  routes only verified datagrams and its real quality array feeds the MLG
  projection. Its typed, read-only Domain accessor is valid only after
  `finish_due`, so the motion lifecycle branch can read the same verified
  Domain without breaking exclusive RX ownership. With a prepared DC sync,
  `receive_with_dc` multiplexes Domain/DC on one master RX call, finalizes
  both on missing responses and port errors, and returns a conservative
  budget-missed report plus the original transport error on port failure.
  Treat a preflight conflict or generation mismatch as a caller safety
  failure; the rejected call has not started RX. The caller still owns
  matching output plans, lifecycle output submission, and the full-cycle
  deadline. `receive_with_dc_and_control` may share the same RX poll with
  in-flight `ControlRequestPool` entries only when all indices are disjoint
  from Domain, DC, and other in-flight controls. The control consumer must
  match the request's index and command as well as the master-verified
  response; a frame slot ID alone never identifies its datagram owner.
  With the control-enabled receive entry, report newly expired requests after
  the bounded RX finishes, including link-down and port-error returns; reap
  their RX indices with the same port clock before returning. Skip the extra
  index sweep when no request newly expired. The caller still owns control
  service-state progression and the final cycle deadline.
- Prepare DC and at most one control TX through the scheduled bank only after
  validating the current master cycle, absolute deadlines, the control
  request's owned generation and Prepared state, and the Domain/DC/in-flight
  control index partition. Do not require a control request's generation to
  equal the current cyclic generation; the matching control consumer verifies
  that owned generation when the request remains InFlight across cycles.
  A prepared DC generation must be finalized by the shared RX owner even if
  its send fails. Retire a failed control send as `Failed(TransmitFailed)`;
  an unsent Prepared request remains the service owner's responsibility.
  Project both the reported send failure and post-TX deadline into safety
  facts before allowing motion; neither accepted TX nor a previous DC lock
  establishes current-cycle qualification.
- For startup, mapping, DC configuration, or another non-mailbox service,
  keep the control request owned by its matching service FSM across the
  scheduled transport stage. `Prepared` may be submitted once; `InFlight`
  participates only in the common Domain/DC/control RX and timeout sweep and
  must never be retransmitted. The transport owner records immutable before/
  after request-state evidence, always finalizes RX after a valid submit, and
  returns terminal requests without releasing them. Only the matching service
  may validate action identity, advance business state, and release the slot.
  Before lifecycle output, explicitly select the owned Platform,
  Configuration, Topology, or Drive gate and combine the service FSM's actual
  readiness with transport/request failure; a caller-supplied ready value
  cannot override a failed TX or request. Treat submit/RX invariant errors as
  fault or reinitialization boundaries, not as an invitation to rebuild a
  healthy report from individual sent bits.
- Use `receive_with_dc_and_mailbox` only with the matching mailbox pending
  action and request handle. Reject an index/generation/address/operation/
  length/deadline mismatch, or a changed Prepared/InFlight send payload
  (including its zero-padded read/write wire area),
  before RX so a foreign request cannot fault the
  mailbox FSM. After the common RX finalizes Domain/DC/control, consume only
  terminal Complete/Failed mailbox requests; keep Prepared or InFlight
  requests owned by the caller. Preserve both the full RX report and the
  optional mailbox FSM outcome so callers can project failures into safety
  facts. A rejected TX may take the configured mailbox retry/delay path, but
  successful retry is not evidence that current-cycle motion is safe.
- Prefer `run_dc_and_mailbox_cycle` when the mailbox service is the selected
  cyclic control owner. Submit the already-armed process-Domain frame before
  entry, then let this owner pair DC/mailbox TX with the common RX finalizer.
  After a valid submit it must run RX even when the TX report contains a DC or
  control send failure; otherwise a prepared DC generation can leak into the
  next cycle. Carry forward only the returned `request`: terminal mailbox
  outcomes clear it after service consumption, while a live InFlight request
  remains explicit. A staged RX preflight failure must retain the TX report
  and be treated as a fault/reinitialization boundary. Its post-RX deadline
  observation is an intermediate safety fact, not the final deadline after
  process outputs and lifecycle State/event publication.
- A successfully submitted frame may still await RX after its frame-pool slot
  or DMA descriptor is recycled. Track the exact indices armed by each frame
  along with its RX generation, and on rejected TX or partial arm failure
  cancel only matching armed expectations. Never clear all RX entries by the
  reused slot/descriptor number, or an unrelated accepted response is lost.
  For manually constructed frame-pool frames, use `arm_rx_for_frame` so the
  frame records its own expectations; bare `arm_rx` has no frame owner and
  must be canceled separately if its sender rejects TX.
- For the shared-RX-to-output path, prefer
  `StopCycleContext::run_received_with_outputs_until`: confirm the report
  against the bank's just-finalized cycle and current Domain qualities, the
  context's actual motion Domain instance and master report, and the DC
  completion result before deriving frozen-order quality snapshots. Reject a
  stale, altered, or foreign report before gate mutation or auxiliary TX.
  This does not replace the outer owner's control/DC TX or its projection into
  non-bus safety facts. The supplied deadline is checked again after State and
  event publication.
- When `run_dc_and_mailbox_cycle` owns the service stage, pass its complete
  `ScheduledMailboxCycleReport` to
  `StopCycleContext::run_mailbox_cycle_with_outputs_until`; do not reconstruct
  readiness from separate sent bits or a request slot. Reject inconsistent
  request/progress/TX shapes before gate mutation or process TX. Any service TX
  failure, mailbox error, or `RetryScheduled` result must clear configuration
  readiness for that cycle, and a false post-RX deadline must clear (never
  restore) the caller's budget fact. The lifecycle entry then owns due output,
  State/event publication, and the final post-publication deadline observation;
  task release remains an explicit outer stage.
- Bind an explicit pre-RX process stage with `ScheduledProcessInputs`, never
  ad hoc frame construction. Activation must validate schedule order, exact
  Domain segment coverage, image bounds, index uniqueness, and writable-range
  separation. Preserve the complete `ScheduledProcessTxReport` and pass it with
  the mailbox/DC report to
  `StopCycleContext::run_process_mailbox_cycle_with_outputs_until`; a rejected
  frame or false stage deadline clears budget qualification, while a forged
  cycle/mask/count is rejected before any new TX. Use this stage only when no
  prior lifecycle output already owns the next RX indices (for example initial
  priming or explicit recovery). In steady state, carry the previous output
  stage into the next release; never resubmit an armed Domain index merely to
  satisfy the ordering API.
- In the stable scheduled cycle, use `ScheduledProductionCycleOwner` to make
  `PrimingRequired -> ReceiveArmed -> OutputPending -> ReceiveArmed` explicit.
  Bind priming to the scheduler's expected generation and RX deadline, then
  confirm the same generation through the finalized bank report. Schedule
  auxiliary outputs for the next receive cycle, not the cycle whose inputs
  were just consumed. The next handoff generation must be exactly the wrapping
  successor and its RX deadline must equal the scheduler's expected absolute
  deadline. A partial handoff still owns its accepted RX indices and must be
  drained; it never authorizes duplicate priming. Release the task only when
  the handoff is complete, both final deadline observations pass, and State
  plus lifecycle events were published.
- Expire in-flight control requests only when `now > deadline`, matching the
  RX index boundary. Retain `Failed(Timeout)` until the matching service FSM
  consumes and releases it, and do not let a late completion erase terminal
  diagnostics. A mailbox send retry must honor its configured delay just like
  a receive retry; never release a foreign failed request as though it belonged
  to the pending mailbox action.
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
- Keep the checked-in R2 qualification manifest explicitly unqualified. A
  product claim must resolve every evidence path beneath the repository root,
  verify its SHA-256, and bind the same tested commit, configuration, and HIL
  topology across the build report and qualified Q1/Q2 reports. Require frozen
  per-axis `ControlledStopLimits`, two distinct qualified drive vendors, an
  EtherCAT IO module, CSP/CSV/CST plus configured stop-action HIL coverage, the
  fault matrix, known limitations, and safety/license/source reviews. Reuse the
  build/performance validators; never treat simulator tests, placeholder target
  data, or a structurally valid manifest as product qualification.

## Scenario: Unified Production Service Scheduling

### 1. Scope / Trigger

- Trigger: adding or changing the cyclic selection of startup, mapping, DC
  configuration, or mailbox state machines and their lifecycle projection.

### 2. Signatures

- Core entry: `ScheduledProductionServiceScheduler::run_cycle(...)`.
- Service set: `ScheduledProductionServices<MAX_SLAVES, SMS, FMMUS>`.
- Evidence: `ScheduledProductionServiceCycleReport<E, DOMAINS>`.
- Lifecycle entries: `run_service_cycle_with_outputs_until`,
  `run_process_service_cycle_with_outputs_until`, and
  `ScheduledProductionCycleOwner::complete_service_cycle`.

### 3. Contracts

- Selection priority is fixed: Startup, Mapping, DC Configuration, Mailbox.
- The scheduler owns at most one `RequestHandle`; a live request pins the
  selected service until terminal consumption.
- A carried request must match the selected FSM's pending action in index,
  generation, address, operation, wire length, deadline, and prepared payload.
- `Prepared` may transmit once. If it never reaches the wire, release it and
  report `RebuildRequest`; retain the FSM pending action. Before re-enqueueing,
  compare that action's immutable deadline to the current port clock. An
  expired pending action must run the matching FSM timeout path without
  allocating a new pool slot.
- `InFlight` is never retransmitted. It reports `AwaitingResponse` until the
  shared RX completes it or strict `now > deadline` expiry makes it terminal.
- Control requests retain their own generation across cyclic generations.
  Only `ControlRxConsumer::accepts_prior_generation` may authorize this, and
  only for the same InFlight pool slot/index/generation. Domain and DC
  consumers keep the default current-generation-only rule.
- Startup readiness owns the Topology gate. Mapping, DC configuration, and
  Mailbox readiness own the Configuration gate. Callers do not override this
  mapping or supply a replacement readiness boolean.

### 4. Validation & Error Matrix

- Missing/foreign carried handle -> `RequestMismatch(service)` before TX/RX.
- Pool allocation/release failure -> `Control(ControlError)`.
- Low-level control/mailbox submit or receive invariant -> wrapped typed cycle
  error; do not manufacture partial service evidence. If submit rejects before
  a newly prepared request reaches the wire, release that Prepared handle
  before returning the error; preserve only an already InFlight handle.
- Complete/Failed request with action mismatch -> matching controller faults
  with `ActionMismatch` and releases only that owned handle.
- `Failed(Timeout)` -> matching controller timeout path, preserving the exact
  timeout rather than replacing it with `InvalidState`.
- Faulted selected controller -> `service_ready = false`, recovery `Faulted`,
  and no lower-priority service selection until explicit controller restart.

### 5. Good/Base/Bad Cases

- Good: Mapping request generation 41 completes during cyclic generation 2;
  the control consumer authorizes it, the FSM advances, and the handle frees.
- Base: no service is active; DC/Domain shared RX still runs with `Idle`
  evidence and no service gate is changed.
- Bad: a dropped control response leaves the request InFlight; sending it
  again on the next production cycle is a contract violation.
- Bad: a DC TX failure leaves a newly prepared request in the pool; carrying
  that stale Prepared handle instead of releasing/rebuilding it is a leak.

### 6. Tests Required

- Public integration: Mapping outranks Mailbox, and explicit Mapping restart
  is required before Mailbox can run after a Mapping fault.
- Public integration: a request generation different from the current cyclic
  generation completes only through the matching control consumer.
- Public integration: DC failure yields `RebuildRequest` and zero pool usage.
- Public integration: a rebuilt action that expires before the next cycle
  reaches the matching FSM as Timeout without occupying a pool slot.
- Public integration: submit preflight rejection releases the scheduler-owned
  Prepared handle while retaining the FSM pending action.
- Public integration: dropped response remains InFlight with no control
  retransmit, then expires once and reaches the controller as exact Timeout.
- Lifecycle integration: the unified report clears Topology or Configuration
  according to selected service and is accepted by the stable cycle owner only
  when `ScheduledDomainBank` confirms its underlying RX evidence.

### 7. Wrong vs Correct

#### Wrong

```rust
// Re-enqueues every tick and lets the caller choose a convenient safety gate.
let request = controller.enqueue_pending(&mut pool)?;
let facts = other_cycle_facts_from_control_cycle(&cycle, caller_gate, true, facts);
```

#### Correct

```rust
let cycle = scheduler.run_cycle(/* fixed service set and shared RX owners */)?;
let facts = other_cycle_facts_from_production_service_cycle(&cycle, facts);
```

## Testing Requirements

Run `make ci` before handing off a change. Run `make test-hil` when modifying
the Linux port or simulator. New DMA behavior must cover ownership, cache
ordering, stale handles, invalid lengths, error rollback, and at least one
end-to-end simulator path. New eBPF ABI fields must have fixed-size decode and
invalid-discriminant tests. New CiA 402 PDO fields must have public API tests
for all supported modes and a failure-path test proving the output image is
unchanged.

For eBPF duration evidence, entry/exit observations must use bounded kernel
maps, keep the fixed event size stable, validate complete attach pairs, and
cover both the duration discriminant and its cycle-risk correlation. A local
syntax or unit test does not qualify target-kernel verifier, permission,
pressure-injection, or production hook behavior; those remain explicit
environment-level evidence.

For eBPF network-drop evidence, filter by protocol and optional interface
identity rather than current PID/TID, because receive/drop processing may run
in softirq context. Aggregate in a fixed-capacity map and emit only the first
threshold crossing in a bounded window. An unresolved interface may increment
a diagnostic counter but must not produce `HOST_NIC_DROP`; that incident also
requires a correlated transport-risk cycle. Preserve the fixed event size and
treat target-kernel packet injection/verifier results as separate evidence.

## Code Review Checklist

- Is the worst-case loop bounded by a static capacity or explicit budget?
- Are errors propagated without silently publishing partial process data?
- Are descriptor generation and cache/barrier ordering correct?
- Does the test exercise the public API and the failure path?
- Were `make ci` and the relevant cross-target checks run?
