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
  latched fallback path. ProcBuf v5 carries per-axis requested and issued
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
treat target-kernel packet injection/verifier results as separate evidence. A
hosted virtual-veth unhandled-protocol fixture can qualify the shared
filter/window/ringbuf/incident path, but physical NIC, driver, NAPI, XDP,
qdisc, queue-pressure, congestion, real EtherCAT-device, production-kernel,
overhead, and WCET claims remain separate.

## Scenario: eBPF Network-Drop Runtime Qualification

### 1. Scope / Trigger

- Trigger: add or change `skb:kfree_skb` protocol/ifindex filtering, bounded
  network-drop aggregation, fixed evidence decode, cycle correlation, observer
  health projection, or its hosted qualification.
- Scope: one unique virtual-veth pair receives controlled unhandled EtherType
  frames on one inherited allowed CPU. This does not qualify physical NICs,
  drivers, NAPI, XDP, qdisc, queue pressure, congestion, real EtherCAT devices,
  production kernels, overhead, WCET, or long-running HIL.

### 2. Signatures

- Entry: `make test-ebpf-network-drop-runtime`.
- Fixture: `network_drop_qualification <esop_runtime.bpf.o> <report.json>`.
- Artifact: `build/ebpf_network_drop_qualification.json`.
- Validator: `scripts/validate-ebpf-network-drop-qualification.py <report>`.

### 3. Deterministic Fixture

- Build the BPF object and Rust fixture unprivileged; require `iproute2` and
  elevate only the final fixture with root or passwordless sudo.
- Create two unique, bounded interface names in the current network namespace.
  Disable IPv6 only on those disposable links when their per-interface sysctls
  exist, bring both ends up, and read exact ifindex/MAC/MTU/drop counters from
  sysfs.
- Pin the fixture to the first CPU in its inherited affinity mask. Open one
  `AF_PACKET/SOCK_RAW` socket with protocol zero so injection does not register
  a receive handler for the protocol under test.
- Attach only `skb:kfree_skb`; require the attach; filter exact receive ifindex
  and host-order EtherCAT `0x88a4`; use threshold four and a one-second window.
- Before the formal run, send one wrong-EtherType frame forward and one
  EtherCAT frame in reverse. Both independent receive-drop counters advance,
  while BPF stats, ringbuf, incident retention, loss, and health remain empty.
- Publish a WKC-risk cycle, then send exactly four forward EtherCAT frames.
  Require receive `rx_dropped` delta at least four and no formal reverse delta.

### 4. Required Assertions

- Final `network_drops=4`, `network_unattributed=0`,
  `network_threshold_events=1`, `emitted_events=1`, and `lost_events=0`; all
  unrelated BPF statistics remain zero.
- Exactly one evidence item is `KernelNetwork/NetworkDrop/Error` with zero
  PID/TID/IRQ, selected CPU, receive ifindex, count/value/threshold four,
  positive duration below one second, nonzero bounded kernel reason, and exact
  cycle/transition identity.
- Exactly one Error/confidence-75 `HostNicDrop` recommends `ControlledStop`;
  observer health moves Healthy to Degraded with fault `0x45422001`.
- Drop the runtime, explicitly close the packet socket, delete the veth pair,
  verify both sysfs entries disappear, restore inherited CPU affinity, and only
  then atomically publish the closed-schema report.

### 5. Failure Policy

- Missing `ip`, root/raw-socket/net-admin/BPF permission, tracepoint, complete
  attach, exact controls, drop counters, event/stat counts, or cleanup -> fail
  and publish no qualified report.
- Any control BPF activity, malformed/rejected record, event loss, extra
  incident, wrong classification/cycle/identity, zero reason, or observer drift
  invalidates qualification.
- Do not hard-code a kernel drop-reason enum number and do not weaken exact BPF
  counts into ranges; the independent interface counter is the lower-bound
  cross-check.
- Model `trace_event_raw_kfree_skb.reason` as the named
  `enum skb_drop_reason` in the vendored BTF and keep the BPF value unsigned.
  Modeling it as `int` or `unsigned int` makes Aya 0.13 reject the
  `FIELD_SIGNED` CO-RE relocation because the local `INT` kind cannot match the
  target kernel's `ENUM` kind; Aya poisons that instruction as helper
  `0x0bad2310`. Syntax/compile checks alone are therefore insufficient and the
  hosted verifier/load gate is mandatory.

### 6. Tests Required

- Rust build and Clippy cover veth, affinity, raw-socket, evidence, and RAII
  cleanup paths; BPF syntax and CO-RE compilation cover production code.
- Object BTF inspection must preserve the `skb_drop_reason` enum kind; the
  source must enforce the enum kind and 32-bit width with compile-time asserts,
  and the target-kernel verifier/load is the final relocation check.
- Validator regressions reject missing/unknown/bool fields, invalid interface
  identity, silent-control drift, counter inconsistencies, partial masks,
  stats/loss, incident/evidence semantics, timing/reason, health, and cleanup.
- Dedicated Actions must install clang/iproute2, execute the privileged fixture,
  upload the report, and the downloaded artifact must independently pass the
  repository validator.

For eBPF page-fault evidence, filter by the tracked process and aggregate by a
bounded CPU/process key. Emit only the first count-threshold crossing in a
bounded window, preserve the triggering task and architecture error code, and
require a correlated cycle-risk window before producing `HOST_PAGE_FAULT`.
Do not infer major/minor outcome or handler duration from
`exceptions:page_fault_user`; those require a separate qualified observation
point. Preserve the fixed event size. Hosted x86_64 qualification may prove the
controlled anonymous-page first-write count-to-incident chain, but memory
pressure, major/minor attribution, production kernels, verifier portability,
and overhead remain separate evidence.

## Scenario: eBPF Page-Fault Runtime Qualification

### 1. Scope / Trigger

- Trigger: add or change page-fault BPF filtering, window aggregation, fixed
  evidence decode, cycle correlation, observer health projection, or its host
  qualification.
- Scope: one pre-warmed, CPU-pinned child performs first writes to exactly 16
  isolated anonymous pages on hosted x86_64 Linux. This does not qualify fault
  address/IP, BPF major/minor attribution, handler duration, memory pressure,
  swap/storage behavior, natural workload root cause, product thresholds,
  production kernels, overhead, WCET, or long-running HIL.

### 2. Deterministic Fixture

- Build the BPF object and Rust fixture unprivileged; elevate only the final
  fixture with root or passwordless sudo.
- Before loading BPF, start a separate child, pin it to one allowed `u16` CPU,
  warm the executable/control path, and prepare 16 separate anonymous mappings.
- Each mapping must contain one writable page followed by a `PROT_NONE` guard
  page and must request `MADV_NOHUGEPAGE`; do not touch the writable page before
  the formal injection.
- Attach only `exceptions:page_fault_user`, require that attach, set
  `tracked_pid` to the child TGID, set the count threshold to 16, and provide one
  transport-risk cycle before releasing the child.
- The formal injection performs exactly one first write per page. Keep the child
  blocked after reporting until BPF evidence, stats, incident, and health have
  been captured and the runtime has been dropped, so teardown faults cannot
  contaminate the window.

### 3. Required Assertions

- Child `getrusage(RUSAGE_SELF).ru_minflt` delta is exactly 16.
- `page_faults`, emitted event count, decoded evidence count, evidence value,
  and incident count are exactly 16/1/1/16/16 as applicable; ring-buffer loss is
  zero and all unrelated attach/stat counters remain zero.
- The sole evidence is `KernelMemory/PageFault`, belongs to the child PID on the
  selected CPU, uses the x86_64 user/write/not-present error-code detail, and
  carries the configured threshold/window and correlated cycle.
- The sole incident is Warning/confidence-65 `HostPageFault` with
  `DegradeHostObservation`; the final heartbeat is Degraded with fault
  `0x45422001` and the expected monotonic epoch/sequence transition.
- Atomically write `build/ebpf_page_fault_qualification.json`, validate it with
  the closed-schema validator, and upload it from the dedicated hosted CI job.

### 4. Failure Policy

- Fail closed on unavailable x86_64 architecture, missing tracepoint, attach or
  verifier error, PID/CPU mismatch, any count mismatch, unexpected event,
  ring-buffer loss, wrong classification/action/health transition, malformed
  report, or child protocol/exit failure.
- Do not weaken exact counts to ranges to mask cold-path or teardown faults.
  Move all setup before attach and all cleanup after detach instead.

For eBPF lifecycle hard facts, remember that `sched_process_exit` fires for
threads. Apply the configured process filter to TGID, emit `ProcessExit` only
for `tid == tgid`, and count ignored worker exits without escalating them. Read
OOM identity from the typed `oom:mark_victim` victim PID rather than current
task context; do not invent a TGID when the tracepoint exposes only one PID.
Keep the fixed event ABI stable. A hosted single-process cgroup-v2 fixture may
qualify the raw victim-PID chain when it independently proves one local OOM
kill, but PID namespace/cgroup identity, victim thread-group resolution,
global OOM behavior, production-kernel behavior, and overhead remain separate
qualification evidence.

## Scenario: eBPF Process-Exit Runtime Qualification

### 1. Scope / Trigger

- Trigger: add or change process-exit BPF filtering, fixed evidence decode,
  hard-fact correlation, observer health projection, or its host qualification.
- Scope: one synchronized worker exit and one tracked leader exit on a hosted
  Linux kernel. It does not qualify exit cause, complete thread-group death,
  PID namespaces/cgroups, restart supervision, production kernels, or timing.

### 2. Signatures

- Entry: `make test-ebpf-process-exit-runtime`.
- Fixture: `process_exit_qualification <esop_runtime.bpf.o> <report.json>`;
  `--tracked-child` is a private same-binary pipe protocol mode.
- Artifact: `build/ebpf_process_exit_qualification.json`.
- Validator: `scripts/validate-ebpf-process-exit-qualification.py <report>`.

### 3. Contracts

- Load the production CO-RE object with enabled and required masks both equal
  to `ATTACH_PROCESS_EXIT`. Track the parent until the waiting child exists,
  then update `tracked_pid` before releasing any child thread.
- Join exactly one worker before its acknowledgement. It must increment only
  `thread_exits_ignored`; poll, incident, loss, and health-fault fields remain
  zero and the observation remains Healthy.
- A successful leader exit produces exactly one emitted/process-exit event,
  one `KernelProcess/ProcessExit/Critical` evidence item, one Critical
  `UserComponentExit` with `LatchFault` and confidence 100, and one Failed
  observation with fault `0x45422002`.
- Remove stale report/temp files before building and publish JSON by same-dir
  temporary write plus rename only after every assertion succeeds.

### 4. Validation & Error Matrix

- Missing verifier/permission/tracepoint or partial attach -> fixture fails.
- Child protocol mismatch, early exit, or bounded wait timeout -> fixture kills
  and reaps the child, returns nonzero, and publishes no report.
- Worker record/incident, wrong counter, malformed/rejected evidence, or loss
  -> fixture and validator reject qualification.
- Wrong PID/TID, boot/epoch, kind/severity/action, scalar/cycle fields, or
  observer state/fault/count -> validator rejects the exact-schema report.
- No root/passwordless sudo -> runner fails explicitly after unprivileged build;
  it never converts missing privilege into a skip.

### 5. Good/Base/Bad Cases

- Good: one joined worker is ignored, the leader exits normally, and the
  report proves the exact kernel-to-incident-to-health chain with zero loss.
- Base: source/unit/BPF-syntax checks pass but no privileged report exists;
  implementation is tested, but host runtime qualification is not claimed.
- Bad: count a worker as component death, infer exit code/signal, accept a
  partial mask, retain a stale report after failure, or broaden hosted evidence
  into production-kernel or WCET qualification.

### 6. Tests Required

- Rust all-target tests and Clippy must compile the parent/child fixture.
- Validator regression tests cover valid CLI input, missing/unknown/bool fields,
  partial masks, worker incidents, counts/loss, identity/semantic/cycle drift,
  and observer mismatch.
- `make ci` includes the validator suite. A dedicated privileged Actions job
  runs the host target and uploads the validated report.

### 7. Wrong vs Correct

#### Wrong

Spawn an unsynchronized child, track all processes, sleep for timing, accept
any process-exit record, and label it complete component death.

#### Correct

Track only a waiting child, release and verify the joined worker first, then
release the leader, require exact counters/evidence/health, and preserve the
narrow leader-exit claim in the report and documentation.

## Scenario: eBPF Memcg OOM Runtime Qualification

### 1. Scope / Trigger

- Trigger: add or change `oom:mark_victim` filtering, fixed OOM evidence
  decode, hard-fact correlation, observer health projection, or its host
  qualification.
- Scope: one prepared single-threaded child in a unique hosted cgroup-v2 leaf
  with a fixed `memory.max`. This does not qualify global OOM, victim TGID,
  namespace/cgroup identity, trigger/root cause, container delegation, restart
  supervision, production kernels, timing, WCET, or long-running pressure.

### 2. Signatures

- Entry: `make test-ebpf-oom-runtime`.
- Fixture: `oom_qualification <esop_runtime.bpf.o> <report.json>`;
  `--oom-child` is a private same-binary fixed-pipe mode.
- Artifact: `build/ebpf_oom_qualification.json`.
- Validator: `scripts/validate-ebpf-oom-qualification.py <report>`.

### 3. Contracts

- Create a fresh leaf directly below the cgroup-v2 root, require the memory
  controller, set a fixed 32 MiB hard limit, disable swap when available, and
  keep group OOM disabled. Never limit the parent or runner cgroup.
- Start and warm the child first, move only its exact PID into the leaf, and
  verify sole membership before loading/releasing. The child remains
  single-threaded so the raw victim PID has one expected identity.
- Load the production CO-RE object with enabled and required masks both equal
  to `ATTACH_OOM_KILL`. Baseline BPF and local cgroup OOM counters are zero.
- Release a bounded 128 MiB anonymous-page injection. Require child `SIGKILL`,
  local `oom >= 1`, exact `oom_kill == 1`, and `oom_group_kill == 0`.
- The sole BPF event is `KernelMemory/OomKill/Critical` for the child PID/TID.
  It produces one Critical/confidence-100 `HostOom` with `LatchFault` and a
  Failed observation with fault `0x45422002`, without cycle correlation.
- Reap the child, verify the leaf is empty, remove it, then atomically publish
  and validate the report. Cleanup failure invalidates qualification.

### 4. Validation & Error Matrix

- Missing cgroup v2/memory controller, partial attach, verifier/permission
  failure, early/normal/wrong-signal child exit, timeout, or residue -> fail.
- No local OOM, zero/multiple victim kills, group kill, wrong PID/TID, extra
  event, malformed/rejected record, loss, wrong incident/action/health, or
  report-schema mismatch -> fail and publish no qualified artifact.
- `memory.events.local.oom` and `max` are lower-bound diagnostics, not exact
  portable counts; `oom_kill` is the exact single-victim contract.
- No root/passwordless sudo -> fail explicitly after unprivileged compilation.

### 5. Tests Required

- Rust all-target checks and Clippy compile the cgroup/child fixture.
- Validator tests cover closed schema, geometry, membership, signal/cleanup,
  local counter deltas, masks/stats/loss, victim identity, incident semantics,
  cycle absence, and the Healthy-to-Failed transition.
- `make ci` includes the validator suite. A dedicated privileged Actions job
  runs the host fixture and uploads the validated report.

## Scenario: eBPF CPU Frequency Limit Evidence

### 1. Scope / Trigger

- Trigger: add or change Linux cpufreq policy-limit observation in the BPF
  bundle, Aya runtime, or incident correlator.
- Scope: `power:cpu_frequency_limits` is a policy-bound signal. It must not be
  generalized into instantaneous frequency, thermal cause, or throttle
  residency without a separate qualified source.

### 2. Signatures

- Configuration: `RuntimeConfig::{cpu_frequency_floor_khz,
  cpu_frequency_policy_cpu}`.
- Runtime update: `BpfRuntime::update_cpu_frequency_tracking(floor_khz,
  policy_cpu)` writes one complete `KernelContext` value.
- Attach point: optional `power:cpu_frequency_limits`; it remains outside the
  default required mask.

### 3. Contracts

- `floor_khz` is nonzero. `policy_cpu == u32::MAX` means every policy; exact
  filters must fit the event ABI's `u16` CPU field, including CPU 0.
- The context owns an internal nonzero policy epoch. Every successful runtime
  policy update increments it; BPF episode state rearms when its stored epoch
  differs.
- Evidence writes policy `cpu_id` to `cpu`, policy `max_freq` to
  `observed_value`, the configured floor to `threshold`, and zero PID/TID and
  duration. The fixed event remains 96 bytes.
- One bounded map entry tracks each policy CPU. Emit on the first positive
  `max_freq < floor`; suppress repeated below-floor updates; rearm when the
  policy recovers or the policy epoch changes.
- `HOST_CPU_THROTTLE` requires both `0 < observed_value < threshold` and a
  correlated deadline/WKC/DC-risk cycle. Different policy CPUs must not merge
  into one top-level incident.

### 4. Validation & Error Matrix

- `floor_khz == 0` -> `InvalidConfiguration`; do not update the context map.
- Exact `policy_cpu > u16::MAX` -> `InvalidConfiguration`; `u32::MAX` remains
  the only all-policy sentinel.
- Missing optional tracepoint/program -> reduced attach capability; keep the
  scheduler/process baseline running.
- Bounded state insertion failure or ring-buffer output failure -> increment
  `lost_events`; never block or fabricate evidence.
- Zero maximum, at/above-floor maximum, or healthy cycle -> no incident.

### 5. Good/Base/Bad Cases

- Good: policy CPU 7 changes from 2.4 GHz to a 1.8 GHz maximum with a 2.0 GHz
  floor during a deadline-risk cycle; emit one attributed incident.
- Base: further 1.7/1.6 GHz policy updates in the same episode only increment
  suppression statistics; a later 2.0 GHz update rearms observation.
- Bad: treat an ordinary `power:cpu_frequency` transition, hook execution CPU,
  or a policy maximum as proof of instantaneous throttling cause or duration.

### 6. Tests Required

- Decode asserts policy CPU, actual maximum, configured floor, zero task
  identity, and unchanged 96-byte event size.
- Configuration tests reject zero floor and unrepresentable exact CPU filters
  without mutation, and prove policy epoch advancement on successful updates.
- Correlator tests reject at-floor, zero-maximum, and healthy-cycle evidence,
  and prove different policy CPUs retain distinct incidents.
- C/Rust ABI assertions cover context/stat sizes; statistics aggregation covers
  saturation; BPF syntax and real CO-RE compilation cover the typed record.

### 7. Wrong vs Correct

#### Wrong

Treat every dynamic frequency transition as `HOST_CPU_THROTTLE`, record the
current hook CPU, and emit repeatedly while a policy remains below a floor.

#### Correct

Read typed policy `cpu_id`/`max_freq`, compare against the configured kHz floor,
emit one event per bounded policy episode, and require cycle-risk correlation
before assigning the incident code.

## Scenario: eBPF Scheduler Runqueue-Latency Evidence

### 1. Scope / Trigger

- Trigger: add or change Linux `sched_wakeup`/`sched_switch` runqueue-latency
  observation, exact scheduler-TID filtering, or its privileged host
  qualification.
- Scope: the hosted qualification proves one controlled wake-to-switch interval
  behind an injected FIFO blocker. It must not be generalized into ordinary
  workload behavior, a natural root cause, product priority/CPU-isolation
  policy, WCET, or production-kernel qualification.

### 2. Signatures

- Configuration: `RuntimeConfig::{scheduler_tid,
  scheduler_latency_threshold_ns}` with enabled and required masks exactly
  `ATTACH_SCHED_WAKEUP | ATTACH_SCHED_SWITCH`.
- Qualification entry: `make test-ebpf-scheduler-runqueue-runtime` writes
  `build/ebpf_scheduler_runqueue_qualification.json`; validate it with
  `scripts/validate-ebpf-scheduler-runqueue-qualification.py <report>`.
- Fixture platform calls: `sched_getaffinity`, singleton
  `sched_setaffinity`, process-private `FUTEX_WAIT|FUTEX_PRIVATE_FLAG` and
  `FUTEX_WAKE|FUTEX_PRIVATE_FLAG`, plus verified `SCHED_FIFO` policy.

### 3. Contracts

- Select two distinct CPUs from the process's actual allowed affinity set.
  Keep the controller on CPU B; pin the target and blocker to CPU A.
- The target reports its Linux TID, confirms CPU A, then enters private futex
  sleep. `/proc/self/task/<tid>/stat` must show interruptible sleep before the
  runtime loads the exact-TID policy.
- The blocker moves to CPU A, enters a valid nonzero `SCHED_FIFO` priority,
  publishes readiness, and then only spins on an atomic stop flag. It performs
  no allocation, channel operation, sleep, yield, or logging while active.
- Store one to the target gate and require the futex wake result to be exactly
  one. During the configured 25 ms hold, target completion must remain false;
  after blocker release, both threads must join within bounded deadlines.
- The BPF wakeup hook stores the exact target TID timestamp. The switch hook
  matches `next_pid`, deletes state, and emits only for strict
  `duration_ns > threshold`. Fixed evidence remains 96 bytes with zero PID,
  exact TID, destination CPU A, zero IRQ/ifindex/detail, and count one. Because
  this producer supplies zero as its explicit ID, the shared emitter must set
  `evidence_id == timestamp_ns` to the nonzero emit timestamp.
- Successful qualification has exact statistics `wakeups=1`,
  `scheduler_stalls=1`, `emitted_events=1`, zero migrations/loss, one Error
  `HostSchedulerStall`, `ControlledStop`, confidence 70, and a Healthy-to-
  Degraded heartbeat transition with fault `0x45422001`.
- Delete stale final/temp reports before setup. Publish the report only by a
  same-directory rename after all assertions pass. Every failure path releases,
  wakes, stops, and joins any remaining target or FIFO blocker.

### 4. Validation & Error Matrix

- Fewer than two allowed CPUs, CPU IDs outside `u16`, equal CPUs, affinity
  acknowledgement failure, or target sleep timeout -> nonzero exit and no
  success report.
- Invalid host FIFO range, denied scheduler transition, policy verification
  mismatch, or blocker readiness timeout -> hard prerequisite failure.
- Futex wake count other than one or target completion during the blocker hold
  -> injection failure; do not accept a latency record.
- Partial wakeup/switch attach, nonempty baseline, poll timeout, extra record,
  count drift, malformed/rejected evidence, or any loss -> qualification
  failure.
- Duration below the blocker hold, at/below threshold, at/above the one-second
  sanity maximum, or inconsistent incident/evidence timing -> report rejection.
- Unknown/missing/bool-as-integer fields, wrong TID/CPU/cycle/classification,
  a fallback evidence ID unequal to its emit timestamp, or observer-health
  mismatch -> closed-schema validator failure.

### 5. Good/Base/Bad Cases

- Good: target TID 101 sleeps on CPU A, the CPU-A FIFO blocker becomes ready,
  CPU-B controller wakes one waiter, target stays incomplete for 25 ms, then
  runs after release and produces one matching controlled-stop incident.
- Base: source/unit/BPF-syntax checks pass, but a host without two CPUs,
  `SCHED_FIFO`, tracepoint capability, root/passwordless sudo, or exact counts
  produces no qualification claim.
- Bad: run the controller on CPU A, treat a readiness flag as proof of futex
  sleep, accept `>= 1` records, leave a FIFO spinner alive on error, or call the
  injected interval product scheduler WCET.

### 6. Tests Required

- Rust build and Clippy cover the Linux fixture and RAII cleanup paths; BPF
  syntax and CO-RE compilation cover the production tracepoint programs.
- Closed-schema tests reject missing/unknown/bool fields, partial attachment,
  invalid CPU/TID/sleep/FIFO prerequisites, zero or multiple wakes, early target
  completion, baseline/final count drift, loss, wrong classification/cycle,
  timing inconsistencies, and health-transition mismatches.
- Dedicated Actions must run the privileged fixture, upload
  `ebpf_scheduler_runqueue_qualification.json`, and the downloaded artifact
  must independently pass the repository validator.

### 7. Wrong vs Correct

#### Wrong

Wake a thread under uncontrolled system load, accept any scheduler record, and
describe the result as a production root-cause or WCET qualification.

#### Correct

Isolate one sleeping exact-TID target and one bounded FIFO blocker on CPU A,
control release from CPU B, require exact counts and zero loss, and claim only
the measured hosted-kernel controlled wake-to-switch chain.

## Scenario: eBPF Softirq Duration Runtime Qualification

### 1. Scope / Trigger

- Trigger: add or change IRQ/softirq CPU/vector filtering, duration pairing,
  `HostIrqStorm` identity, or the privileged softirq host qualification.
- Scope: one controlled loopback `NET_RX` vector action on one allowed hosted
  Linux CPU. It does not qualify hard IRQ, NIC/driver/NAPI behavior, product
  interrupt budgets, sustained pressure, production kernels, overhead, or WCET.

### 2. Signatures

- Configuration: `RuntimeConfig::{softirq_duration_threshold_ns,
  interrupt_filter_cpu, interrupt_filter_vector}` with enabled and required
  masks exactly `ATTACH_SOFTIRQ_ENTRY | ATTACH_SOFTIRQ_EXIT`.
- Runtime update: `BpfRuntime::update_interrupt_filter(cpu, vector)` updates
  both selectors through one complete kernel-context map write.
- Qualification entry: `make test-ebpf-softirq-runtime` writes
  `build/ebpf_softirq_qualification.json`; validate it with
  `scripts/validate-ebpf-softirq-qualification.py <report>`.
- Injection: one connected IPv4 loopback `sendmsg` with
  `UDP_SEGMENT=1200`, a 64,800-byte payload, and exactly 54 receives.

### 3. Contracts

- Keep the 176-byte C/Rust kernel-context ABI stable by assigning the existing
  reserved `u16`/`u32` fields to CPU/vector filtering. `u16::MAX` and
  `u32::MAX` mean all CPUs/vectors and preserve existing behavior.
- Read the actual process affinity, choose an allowed CPU representable by the
  evidence ABI after a bounded `/proc/softirqs` quiet sample, pin the fixture,
  and verify the active CPU before loading the runtime.
- Apply CPU/vector filters on entry before inserting a start timestamp. A
  filtered event creates no map state or statistics; exit naturally ignores
  it. Cycle updates must preserve both selectors.
- CPU/vector matching is spatial filtering, not temporal isolation. The hosted
  qualifier must load each fresh runtime with an exact impossible softirq
  vector, open vector 3 only after the pre-injection `NET_RX` counter read, and
  close it immediately after the post-send counter read, before receive/poll.
  `u32::MAX` remains the production all-vector sentinel and must not be used as
  the closed value. Record and validate both phases' closed/open/closed states.
- Use two fresh runtimes and agents. Calibration uses threshold one nanosecond
  and must produce exactly one record/incident/sample/overrun/emission with
  zero loss. Formal threshold is
  `max(1, calibration_duration_ns / 8)` and the formal duration must be
  strictly greater than it.
- Both injections must send the full payload, receive exactly 54 datagrams of
  1,200 bytes, and advance the selected CPU's `NET_RX` count by exactly one.
  `/proc/softirqs` supports the claim but does not replace BPF CPU/vector proof.
- Formal evidence is one Error `KernelIrq/SoftirqCpuTime` item with fixture
  PID/TID, exact CPU, vector 3, zero ifindex/detail, formal cycle/transition,
  count one, and equal positive observed/duration values. PID/TID is execution
  context only and must not be described as softirq ownership or root cause.
- Correlation yields one Error `HostIrqStorm`, `ControlledStop`, confidence 70,
  one retained evidence item, and a Healthy-to-Degraded heartbeat transition
  with fault `0x45422001`. Merge identity includes interrupt kind, CPU, and
  vector so hard IRQ and softirq or different CPU/vector pairs remain distinct.
- Formal statistics are exact: one softirq sample, overrun, and emission; zero
  hard-IRQ samples/overruns, loss, malformed/rejected evidence, newly reported
  loss, and dropped incidents. Publish only by same-directory rename after all
  assertions pass.

### 4. Validation & Error Matrix

- Missing softirq pair, partial capability, invalid/unchanged gate state, CPU
  sentinel use, affinity mismatch, unavailable UDP GSO, incomplete send/receive,
  or failure to close after an opened gate -> hard prerequisite failure and no
  final report.
- More or fewer than one target-CPU `NET_RX` execution, nonempty baseline,
  extra/missing ringbuf record, count drift, or any loss -> qualification
  failure; do not retry with weakened counts.
- Calibration duration at/below one nanosecond, incorrect divisor/formula, or
  formal duration at/below the derived threshold -> report rejection.
- Wrong PID/TID/CPU/vector/cycle/classification, inconsistent timestamps or
  duration, hard-IRQ statistics, or observer-health mismatch -> closed-schema
  validator failure.
- Unknown/missing fields, booleans used as integers, stale final/temp output,
  or a non-atomic success artifact -> fail closed.

### 5. Good/Base/Bad Cases

- Good: select allowed CPU 7, pin and verify it, inject one 54-segment GSO
  payload while the filter transitions closed/vector-3/closed, observe exactly
  one vector-3 action, and produce one matching controlled-stop incident with
  zero loss.
- Base: source/unit/BPF-syntax checks pass, but a host without BPF permission,
  tracepoints, UDP GSO, root/passwordless sudo, or exact counts produces no
  qualification claim.
- Bad: count all CPUs, leave vector 3 open during setup/receive/poll, filter only
  at exit, reuse calibration runtime state, accept `>= 1` records, infer
  ownership from current PID/TID, or call the loopback threshold a product
  interrupt budget.

### 6. Tests Required

- C/Rust ABI assertions cover the 176-byte context; unit tests cover unfiltered
  defaults, filter updates, cycle preservation, and incident identity by kind,
  CPU, and vector. BPF syntax and CO-RE compilation cover entry filtering.
- Fixture build and Clippy cover affinity/socket/cleanup paths. Closed-schema
  tests reject filter/attach, closed/open/closed gate state, CPU/vector, GSO,
  calibration, counter/loss, identity/cycle/timing, and health-transition
  failures.
- Dedicated Actions must run the privileged fixture, upload
  `ebpf_softirq_qualification.json`, and the downloaded artifact must
  independently pass the repository validator.

### 7. Wrong vs Correct

#### Wrong

Send arbitrary loopback packets, accept any softirq event on any CPU, and
describe current task identity or a hosted calibration threshold as causality
or a production budget.

#### Correct

Filter one allowed CPU, open vector 3 only around the counter-bounded GSO send,
close it before receive/poll, use isolated calibration and formal runs, require
one complete GSO/NET_RX/event/incident chain with zero loss, and claim only
controlled hosted loopback qualification.

## Scenario: eBPF Scheduler Migration Evidence

### 1. Scope / Trigger

- Trigger: add or change Linux scheduler-migration observation in the BPF
  bundle, Aya runtime, or incident correlator.
- Scope: `sched:sched_migrate_task` is a scheduler entity movement fact. It
  must not be generalized into measured stall duration, affinity failure,
  migration cause, or cache/NUMA impact without a separately qualified source.

### 2. Signatures

- Configuration: `RuntimeConfig::{tracked_pid, scheduler_tid,
  scheduler_latency_threshold_ns, scheduler_migration_threshold,
  scheduler_migration_window_ns}`.
- Runtime update: `BpfRuntime::update_scheduler_tracking(tid,
  latency_threshold_ns, migration_threshold, migration_window_ns)` writes one
  complete `KernelContext` value.
- Attach point: optional `sched:sched_migrate_task`; it remains outside the
  default required mask.
- Qualification entry: `make test-ebpf-scheduler-migration-runtime` writes
  `build/ebpf_scheduler_migration_qualification.json`; the validator is
  `scripts/validate-ebpf-scheduler-migration-qualification.py <report>`.

### 3. Contracts

- A nonzero `scheduler_tid` is the exact scheduler entity filter. Zero falls
  back to `tracked_pid`; two zero values preserve all-task development mode.
- Migration threshold fits `u32`, and threshold/window are nonzero. The context
  owns a nonzero epoch; every successful runtime policy update increments it.
- One fixed-capacity LRU entry per TID stores a monotonic count window. Missing,
  expired, backwards-time, or stale-epoch state starts at one. Emit only when
  the count first reaches the threshold; saturate rather than wrap.
- Fixed evidence remains 96 bytes. It writes zero PID, scheduler entity to TID,
  destination CPU to `cpu`, origin CPU to the kind-specific `irq` slot, count
  to `observed_value` and `count`, count threshold to `threshold`, elapsed
  window time to `duration_ns`, and saturated priority to `detail`.
- Runqueue-latency evidence must write `sched_switch.next_pid` as TID rather
  than the current hook task.
- `HOST_SCHEDULER_STALL` from migration requires a consistent count at or above
  threshold plus a correlated deadline/WKC/DC-risk cycle. It remains lower
  confidence than measured runqueue latency.
- The privileged fixture requires two CPUs from its real allowed affinity set,
  loads the attached runtime with the valid but unreachable `i32::MAX` inert
  scheduler TID, pins one runnable worker to CPU A, then atomically replaces
  the inert policy with the exact worker TID before forcing singleton-mask
  A-to-B-to-A movement. Never leave `scheduler_tid=0` during setup: its
  `tracked_pid` fallback can count unrelated leader migrations and contaminate
  the exact-zero baseline. The first controlled move must emit nothing; the
  second must produce the only evidence and incident.
- A successful qualification has exact statistics `scheduler_migrations=2`,
  `scheduler_migration_threshold_events=1`, `emitted_events=1`, zero loss, a
  Warning/confidence-60 `HostSchedulerStall` with `DegradeHostObservation`, and
  a Degraded heartbeat with fault `0x45422001`.

### 4. Validation & Error Matrix

- Zero latency threshold, zero/oversized migration threshold, or zero window ->
  `InvalidConfiguration`; do not update the context map or local mirror.
- Nonpositive PID, negative CPU, equal origin/destination, or CPU above `u16` ->
  ignore the malformed tracepoint record.
- Missing optional tracepoint/program -> reduced attach capability; keep the
  required wakeup/switch/process baseline running.
- Bounded state insertion failure or ring-buffer output failure -> increment
  `lost_events`; never block or fabricate evidence.
- Below-threshold migration, inconsistent count fields, or healthy cycle -> no
  incident.
- Fewer than two allowed CPUs, a non-acknowledged destination, any setup/extra
  migration, verifier/attach/poll failure, report mismatch, or unavailable
  root/passwordless sudo -> nonzero qualification exit and no success report.

### 5. Good/Base/Bad Cases

- Good: tracked TID 101 migrates four times within 1 ms, last moving CPU 2 -> 7,
  while cycle 42 has a deadline miss; emit one lower-confidence scheduler
  incident with the raw migration evidence.
- Good qualification: a runnable worker starts on allowed CPU A, moves A -> B
  once with no record, then B -> A to cross threshold two. The report preserves
  worker TID, B as raw origin, A as destination, cycle 42/transition 9, and the
  Healthy-to-Degraded observation transition.
- Base: one ordinary migration records state and statistics only; later moves
  in the same window emit once at the threshold and not again.
- Bad: attribute the tracepoint current task as the migrated entity, call every
  migration a stall, or describe origin CPU in raw migration evidence as an
  interrupt vector.

### 6. Tests Required

- Decode asserts zero PID, exact TID, source/destination CPU, count, threshold,
  elapsed window, priority, append-only discriminant, and unchanged 96-byte
  event size.
- Configuration tests reject zero/oversized threshold and zero window without
  mutation, and prove epoch advancement on successful updates.
- Correlator tests reject below-threshold, inconsistent-count, and healthy-cycle
  evidence and prove migration confidence/action are weaker than runqueue
  latency.
- C/Rust ABI assertions cover context/stat sizes; statistics aggregation covers
  saturation; BPF syntax and real CO-RE compilation cover the typed record.
- The closed-schema report tests reject missing/unknown/bool fields, partial
  attachment, invalid/same CPUs, setup or first-move emissions, count/loss
  drift, wrong TID or B-to-A endpoint, wrong cycle/classification, invalid
  duration/priority width, and observer-health mismatches. Dedicated Actions
  must run the privileged target and upload the validated JSON report.

### 7. Wrong vs Correct

#### Wrong

Emit every `sched_migrate_task`, use current PID/TID, and label the event a
measured scheduler stall without cycle evidence.

#### Correct

Filter the typed scheduler entity, count migrations in a bounded epoch-aware
window, emit the first threshold crossing with source/destination identity, and
require transport-risk correlation before assigning the existing incident code.

## Scenario: Runtime Incident Protobuf and Zenoh Projection

### 1. Scope / Trigger

- Trigger: add or change `esop-ebpf-agent::RuntimeIncident`, its Protobuf v1
  projection, diagnostic publication, or typed-query incident validation.
- Scope: host/supervision only. Protobuf and Zenoh must not enter the EtherCAT
  cycle, MLG decision, ProcBuf writer, or fixed agent ABI.

### 2. Signatures

- Projection:
  `project_runtime_incident(&esop_ebpf_agent::RuntimeIncident) -> Result<esop_proto::v1::RuntimeIncident, IncidentAdapterError>`.
- Contract validation:
  `validate_runtime_incident_message(&esop_proto::v1::RuntimeIncident, Option<u64>) -> Result<(), IncidentContractError>`.
- Production publication:
  `ZenohGateway::publish_agent_incident(&esop_ebpf_agent::RuntimeIncident) -> Result<(), RuntimeError>`.
- Query encoding:
  `encode_query_reply(QueryReply, &QuerySelector) -> Result<Vec<u8>, QueryAdapterError>`
  validates every returned incident against the requested boot before encoding.

### 3. Contracts

- Project through the single `runtime_incident` adapter. Validate nonzero
  incident/boot/epoch identity, confidence, ordered time/cycle windows,
  `1..=MAX_INCIDENT_EVIDENCE`, evidence identity, shared boot/epoch, and
  inclusive evidence membership before allocating or publishing.
- Build the external incident ID from boot ID, agent epoch, and agent-local ID.
  Preserve full-width values in additive fields; the legacy 32-bit evidence
  value saturates and never wraps. Existing/reserved Protobuf tags are never
  renumbered or reused.
- Keep reason/kind/domain as stable agent discriminants, use explicit severity,
  action, and affected-component mappings, and do not serialize Rust debug
  output or infer a stronger root cause.
- Query validation repeats the Protobuf incident contract against the requested
  boot. `after_sequence` applies only to `RobotState.sequence`; incident paging
  remains provider-owned and must not reinterpret `cycle_sequence` as a state
  cursor.
- `publish_agent_incident` projects before starting the transport publication,
  and reports projection failures separately from schema, route, and Zenoh
  transport failures.

### 4. Validation & Error Matrix

- Zero incident, boot, or epoch identity -> `IncidentAdapterError::InvalidIdentity`;
  allocate and publish nothing.
- Confidence above 100, reversed time/cycle windows, or zero evidence window ->
  the corresponding typed adapter error; allocate and publish nothing.
- Empty or over-capacity evidence, zero evidence ID, boot/epoch mismatch, or an
  evidence timestamp/cycle outside the inclusive incident window -> the
  corresponding typed adapter error; allocate and publish nothing.
- Query incident boot mismatch -> `QueryAdapterError::BootMismatch`; malformed
  identity, windows, or evidence -> `QueryAdapterError::Incident` and stable
  `invalid_incident` reply behavior.
- Protobuf validation, route, or Zenoh failure after successful projection ->
  preserve the existing schema/route/transport `RuntimeError`; do not relabel
  it as an adapter failure.

### 5. Good/Base/Bad Cases

- Good: a scheduler-latency incident with two ordered records from the same
  boot and agent epoch projects to one deterministic external ID, exact 64-bit
  values, `host.scheduler`, and the explicit controlled-stop action.
- Base: a value at or below `u32::MAX` is equal in legacy and additive fields;
  a larger value saturates only the legacy field while the additive field
  preserves the original value.
- Good query: an incident can have a cycle sequence older than the state
  `after_sequence` cursor and is still returned when its boot and contract are
  valid.
- Bad: serialize `Debug` output, truncate a 64-bit value, accept mixed epochs,
  reinterpret incident cycle as state paging, or publish a partially validated
  incident.

### 6. Tests Required

- Cover every agent rejection class, all stable enum/string mappings,
  deterministic ID disambiguation, 64-bit preservation and legacy saturation.
- The frozen v1 reader must preserve the legacy subset and ignore additive
  fields; the current reader must see the full message.
- Query tests cover cross-boot, cross-epoch, invalid windows/evidence and an
  incident whose cycle is older than the state cursor.
- The loopback router test must publish through `publish_agent_incident`, decode
  after transport, and verify ID, action, provenance, evidence and full values.

### 7. Wrong vs Correct

#### Wrong

Construct Protobuf incidents at each call site, copy only legacy fields, infer
root cause from free-form details, and let query or transport paths discover
identity and evidence inconsistencies after allocation or publication starts.

#### Correct

Use one host-only adapter, reject the complete agent contract before publish,
map every external enum/string explicitly, preserve additive full-width
provenance, revalidate query-provider output against the requested boot, and
keep incident paging independent from robot-state sequence cursors.

## Scenario: eBPF Gateway Operation Stall Evidence

### 1. Scope / Trigger

- Trigger: add or change Zenoh gateway publish/callback markers, gateway
  uprobes, gateway tracking policy, fixed evidence decode, or gateway-stall
  classification.
- Scope: this contract covers the complete asynchronous `publish` operation
  and synchronous command-subscription and query callback invocation. It does
  not qualify Zenoh internal queueing, ROS2, recorder, live transport behavior,
  production target kernels, or production overhead.
- The privileged host qualification covers the shared marker/load/attach/
  ring-buffer/correlator path using bounded direct-marker delays for both
  operation classes. It does not convert those calls into live Zenoh transport,
  router queue, IPC, serialization, permit, reconnect, WCET, or production
  realtime evidence.

### 2. Signatures

- Stable marker ABI:
  `esop_zenoh_gateway_publish_begin_v1(request_id, route_kind)` and
  `esop_zenoh_gateway_publish_end_v1(request_id, route_kind, outcome)`, plus
  `esop_zenoh_gateway_callback_begin_v1(request_id, route_kind)` and
  `esop_zenoh_gateway_callback_end_v1(request_id, route_kind, outcome)`.
- Configuration: `RuntimeConfig::gateway_stall_threshold_ns` and
  `BpfRuntime::update_gateway_tracking(threshold_ns)`.
- Attachment: `BpfRuntime::attach_gateway_publish_probes(target, pid,
  required)` and `BpfRuntime::attach_gateway_callback_probes(target, pid,
  required)` attach each exact marker-symbol pair transactionally.
- Qualification entry: `make test-ebpf-gateway-runtime`; successful execution
  writes `build/ebpf_gateway_qualification.json`, which is independently
  checked by `scripts/validate-ebpf-gateway-qualification.py`.

### 3. Contracts

- Publish and callback request IDs share one nonzero process-local atomic
  sequence. Route, operation class, and outcome are fixed integers; no user
  string is parsed in BPF.
- The publish guard begins immediately before the awaited Session put and ends
  exactly once after success, transport failure handling, or future
  cancellation. The callback guard brackets the gateway-owned command/query
  invocation and ends once after normal completion or Rust unwind; typed query
  decode/provider/encode/reply work stays inside that invocation.
- One 1024-entry LRU map tracks `{TGID, request_id}`. State includes monotonic
  start time, nonzero policy epoch, start TID, route, and operation class.
  Publish accepts only State/Event/Diagnostic; callback accepts only
  Command/Query and completed/abandoned outcomes. Every matched end deletes
  state before class, route, outcome, epoch, or emission validation.
- Context updates are staged and atomically published. Zero threshold is
  rejected without mutation; successful threshold or tracked-PID changes
  advance the gateway epoch.
- The fixed event remains 96 bytes. It writes request ID to `evidence_id`,
  TGID to PID, matching worker TID or zero after migration, elapsed time to
  both `observed_value` and `duration_ns`, configured threshold to `threshold`,
  and route/outcome to bounded detail.
- `GatewayStall` classification requires nonzero threshold, strict
  `duration_ns > threshold`, consistent observed duration, and a correlated
  deadline/WKC/DC-risk cycle.
- The qualification disables unrelated tracepoints, tracks its own PID,
  requires all four gateway attach bits, publishes the same risk cycle to
  kernel and agent, and bounds polling by iteration count and elapsed time. It
  may publish `qualified` only after one Diagnostic/success publish and one
  Command/completed callback have produced two classified records merged into
  one count-two incident with zero mismatch/loss.

### 4. Validation & Error Matrix

- Zero threshold or nonpositive explicit attach PID ->
  `InvalidConfiguration`; do not mutate the context or capability snapshot.
- Missing optional program/symbol/target -> `Ok(false)` with the kernel
  baseline intact; required mode returns the typed failure.
- Second-probe failure -> detach the first link and publish neither attach bit.
- Duplicate/missing request, invalid route/outcome, backwards time, or stale
  epoch -> delete matched state where possible, increment mismatch statistics,
  and emit no incident.
- Map insertion or ring-buffer failure -> bounded diagnostics/loss accounting;
  never block the gateway or control path.
- Missing root/passwordless sudo, verifier/load failure, partial pair attach,
  poll timeout, malformed/rejected evidence, wrong merge/detail/request ID, or
  report-schema mismatch -> nonzero qualification exit and no success artifact.

### 5. Good/Base/Bad Cases

- Good: request 77 publishes diagnostics for 1.5 ms against a 1 ms threshold,
  completes on another worker during cycle 42 with a deadline miss, and emits
  one PID-attributed/TID-zero gateway incident. A command callback that exceeds
  the threshold during the same risk cycle emits the same fixed evidence with
  callback class and Command route validation.
- Good qualification: a 25 ms Diagnostic/success publish and 25 ms
  Command/completed callback against a 5 ms threshold produce request IDs 101
  and 202, complete four-bit attachment, two begin/completion/stall counts, and
  one merged count-two incident with zero mismatch/loss.
- Base: an at-threshold publish or callback completes and removes state without
  evidence; publish cancellation and callback unwind each close marker state
  exactly once.
- Bad: attach a return probe directly to Rust `async fn publish`, call enum-only
  evidence a stall, cross-complete publish state with a callback end, preserve
  a stale TID after worker migration, or leave the first uprobe attached after
  the second fails.

### 6. Tests Required

- Exact-symbol integration links and calls all four versioned C ABI markers.
- Marker tests cover shared concurrent nonzero unique IDs, one terminal call,
  publish cancellation, and callback completion/unwind abandonment.
- Decode tests assert request ID, callback route/outcome, PID/TID, duration,
  threshold, discriminants, and unchanged 96-byte event size. BPF source review,
  syntax checking, and CO-RE object compilation cover operation-class and route
  separation in the shared map.
- Configuration tests reject zero without mutation and prove epoch advancement;
  C/Rust context and statistics sizes must match.
- Correlator tests accept one valid risk-cycle stall and reject at/below
  threshold, inconsistent duration, zero threshold, and healthy-cycle cases.
- Statistics aggregation saturates; BPF syntax and CO-RE object compilation
  cover the bounded map and x86_64 marker argument ABI.
- The report validator accepts the privileged baseline and rejects missing,
  unknown, bool-as-int, partial-pair, wrong route/outcome, duplicate ID,
  at-threshold, inconsistent-duration, mismatch, loss, and wrong-incident data.

### 7. Wrong vs Correct

#### Wrong

Measure only async future construction with uretprobe, leave callback work
unbracketed, attach one marker at a time, and classify every `GatewayStall`
discriminant as a root cause.

#### Correct

Bracket the awaited publish operation and synchronous callback invocation with
separate stable begin/end markers, attach each pair transactionally, validate
operation class in bounded request state, and require internally consistent
over-threshold duration plus transport-risk correlation.

## Scenario: eBPF Linux Raw-Port Syscall Stall Evidence

### 1. Scope / Trigger

- Trigger: add or change Linux raw-port markers, raw-port uprobes, threshold
  policy, fixed evidence decode, or raw-port-stall classification.
- Scope: this contract covers only valid TX `send(2)` and every nonblocking
  `recv(2)` syscall in `LinuxRawPort`. It does not qualify caller scheduling,
  driver queues, NAPI/IRQ, NIC DMA, wire, slave response, WKC, or full cycles.
- The privileged host qualification covers the shared marker/load/attach/
  ring-buffer/correlator path using a blocking Unix-domain `recv(2)` fixture.
  It does not convert that fixture into AF_PACKET, hardware, WCET, or production
  realtime evidence.

### 2. Signatures

- Stable marker ABI:
  `esop_linux_raw_port_operation_begin_v1(ifindex, operation)` and
  `esop_linux_raw_port_operation_end_v1(ifindex, operation, outcome)`.
- Configuration: `RuntimeConfig::raw_port_stall_threshold_ns` and
  `BpfRuntime::update_raw_port_tracking(threshold_ns)`.
- Attachment: `BpfRuntime::attach_raw_port_probes(target, pid, required)`
  attaches the exact marker-symbol pair transactionally.
- Qualification entry: `make test-ebpf-raw-port-runtime`; successful execution
  writes `build/ebpf_raw_port_qualification.json`, which is independently
  checked by `scripts/validate-ebpf-raw-port-qualification.py`.

### 3. Contracts

- TX validation completes before begin. The marker guard brackets the syscall,
  captures `errno` before end on failure, emits one terminal outcome, and adds
  no allocation, lock, sleep, log, clock read, or observer wait.
- Operation and outcome are bounded integers: TX success/error/partial and RX
  frame/empty/link-down/error. Simulator and no-std core behavior stay intact.
- One 1024-entry LRU map tracks each `pid_tgid`. State contains monotonic start,
  policy epoch, interface, and operation. A duplicate begin replaces stale
  state with mismatch accounting. Every matched end deletes state before
  validating interface, operation, outcome, epoch, or emission.
- Context updates are staged and atomic. Zero threshold is rejected without
  mutation; successful threshold or tracked-PID changes advance raw-port epoch.
- The fixed event remains 96 bytes. It writes begin timestamp to evidence ID,
  TGID/TID and end CPU, interface index, elapsed duration/threshold, count one,
  and operation/outcome detail.
- `RawPortStall` classification requires a nonzero threshold, strict
  `duration_ns > threshold`, consistent observed duration, and a correlated
  deadline/WKC/DC-risk cycle before assigning `HOST_PORT_STALL`.
- The qualification disables unrelated tracepoints, tracks its own PID,
  requires both raw-port attach bits, publishes the same risk cycle to kernel
  and agent, and bounds polling by iteration count and elapsed time. It may
  publish `qualified` only after one expected incident, one begin/completion/
  stall, and zero mismatch/loss have all been observed.

### 4. Validation & Error Matrix

- Zero threshold or nonpositive explicit attach PID ->
  `InvalidConfiguration`; do not mutate context or capability state.
- Missing optional program/symbol/target -> `Ok(false)` with all prior
  capability intact; required mode returns the typed failure.
- Second-probe failure -> detach the first link and publish neither attach bit.
- Missing end state, duplicate begin, invalid interface/operation/outcome,
  backwards time, or stale epoch -> mismatch statistics and no incident.
- Map/ring-buffer failure -> bounded loss accounting; never block the port.
- Missing root/passwordless sudo, verifier/load failure, partial attach, poll
  timeout, malformed/rejected evidence, or report-schema mismatch -> nonzero
  qualification exit and no success artifact.

### 5. Good/Base/Bad Cases

- Good: TX `send(2)` on ifindex 7 takes 1.5 ms against a 1 ms threshold during
  a WKC-risk cycle; emit one attributed controlled-stop recommendation.
- Good qualification: a 25 ms Unix receive against a 5 ms threshold produces
  one RX/frame `HostPortStall` for the fixture PID/TID and synthetic ifindex 7,
  with complete attach masks and zero mismatch/loss.
- Base: an at-threshold RX Empty call deletes state and emits no evidence.
- Bad: include frame validation/full cycle in the duration, infer a driver or
  slave root cause, leave stale state after an invalid end, or add a userspace
  clock read to the activated port.

### 6. Tests Required

- Exact-symbol integration links both markers; unit tests freeze operation and
  outcome values and prove one terminal guard call plus unwind fallback.
- Decode asserts evidence ID, PID/TID, CPU, ifindex, duration, threshold,
  detail, append-only discriminant, and unchanged 96-byte event size.
- Configuration tests reject zero without mutation and prove epoch advancement;
  C/Rust context and statistics sizes must match and aggregation must saturate.
- Correlator tests reject at-threshold, inconsistent, zero-threshold, and
  healthy-cycle evidence. BPF syntax and real CO-RE compilation cover the map,
  x86_64 marker argument ABI, outcome masks, and state deletion path.
- The privileged CI job must execute the real object and exact marker pair,
  validate the JSON artifact, and upload it. Validator regression tests reject
  missing/unknown/mistyped fields, booleans as integers, partial pairs,
  inconsistent timing, wrong incident semantics, loss, and mismatch.

### 7. Wrong vs Correct

#### Wrong

Time the complete EtherCAT cycle in userspace, reuse gateway threshold/state,
and call an over-threshold syscall proof of NIC or slave failure.

#### Correct

Bracket only raw socket syscalls with stable no-op markers, track each thread
in an independent bounded epoch-aware map, attach the pair transactionally,
and require internally consistent duration plus transport-risk correlation.
Use the Unix-socket fixture only to qualify the shared observation chain, and
keep AF_PACKET/NIC/driver/wire/slave and realtime-performance claims separate.

## Scenario: eBPF Observability Degradation Runtime Qualification

### 1. Scope / Trigger

- Trigger: change eBPF capability classification, event-loss health projection,
  agent restart semantics, ring-buffer statistics, or hosted degradation
  qualification.
- Scope: qualify one production 4 MiB ring-buffer saturation/loss path and one
  runtime unload/reload recovery on privileged hosted x86_64 Linux.
- Missing BTF, ringbuf, required attach, verifier, and permission outcomes are
  deterministic policy-model cases. Do not describe them as hosted fault
  injection unless a separate fixture actually mutates the host environment.

### 2. Signatures

- Health inputs: `RuntimeAgent::health_mut().set_capability_snapshot(...)`,
  `RuntimeAgent::restart(new_epoch)`, and the runtime poll bridge that calls
  `AgentHealth::record_event_loss(newly_reported_lost_events)`.
- Runtime setup: `RuntimeConfig` enables and requires only
  `ATTACH_PAGE_FAULT`, tracks the fixture TGID, and uses page-fault threshold 1
  with a 1 ns window.
- Qualification entry: `make test-ebpf-observability-degradation-runtime`.
  Success writes `build/ebpf_observability_degradation_qualification.json` and
  validates it with
  `scripts/validate-ebpf-observability-degradation-qualification.py`.

### 3. Contracts

- Keep the shipped ring buffer at `1 << 22`; do not add a smaller test-only map.
  Touch and unmap one fixed 64 MiB anonymous chunk per batch, with at most eight
  batches and no record poll before kernel `lost_events > 0`.
- Saturation must advance page-fault and successful-emission statistics and
  produce positive kernel loss. One bounded poll consumes one record and
  reports the exact new loss clamped to `u32`.
- Positive loss sets fault `0x45421004` and is sticky for the current epoch.
  Reapplying a complete capability snapshot must preserve Degraded state and
  the same loss count.
- Drop runtime 1 before restart. Epoch 2 starts in Restarting with zero attach,
  loss, incident, and fault fields. Only runtime 2's complete snapshot may
  restore Healthy.
- Runtime 2 must accept at least one new-epoch page-fault record with zero new
  loss, malformed records, evidence rejection, and incidents. Publish the flat
  closed-schema report by same-directory rename only after both runtimes detach.

### 4. Validation & Error Matrix

- Missing BTF/ringbuf -> Degraded with capability fault `0x45421001`.
- Missing required page-fault attach -> Degraded with attach fault
  `0x45421003`.
- Missing verifier or BPF permission -> Failed with load fault `0x45421002`.
- Zero saturation loss, early poll, more than eight batches, count mismatch,
  nonsticky same-epoch health, stale restart state, recovery loss/rejection, or
  incomplete cleanup -> nonzero fixture/validator exit and no qualification.
- Missing root/passwordless sudo, non-x86_64 host, unavailable BTF/attach, or
  verifier/load failure -> build may complete, but no hosted claim is emitted.

### 5. Good/Base/Bad Cases

- Good: bounded first-write batches fill the production ring buffer; one poll
  projects the exact delta, epoch 1 stays Degraded, epoch 2 reload recovers and
  accepts fresh evidence.
- Base: `record_event_loss(0)` changes nothing, and a complete capability
  snapshot with no epoch-local loss is Healthy.
- Bad: shrink the ring buffer, poll while filling it, loop or allocate without
  a fixed bound, use a synthetic setter as saturation evidence, or preserve old
  attach/loss/fault state across restart.

### 6. Tests Required

- Agent unit tests assert dedicated loss fault, saturating accumulation,
  same-epoch stickiness, Failed precedence, restart reset, and capability-state
  classification.
- Public ProcBuf/MLG integration proves loss-induced Degraded observation stops
  motion and cannot be healed by a same-epoch capability refresh.
- Validator mutation tests reject unknown/missing/mistyped fields, booleans as
  integers, geometry/mask drift, zero or inconsistent loss, wrong health
  sequences, stale restart fields, failed recovery, and incomplete cleanup.
- CI builds the production BPF object and Rust fixture without elevation,
  elevates only fixture execution, validates the report as the invoking user,
  uploads the artifact, and the downloaded artifact must pass the same validator.

### 7. Wrong vs Correct

#### Wrong

Reduce map capacity or call `record_event_loss` directly, then claim a real
kernel saturation and missing-capability environment; allow a healthy snapshot
to clear loss or carry old runtime state into the new epoch.

#### Correct

Fill the production nonblocking ring buffer with bounded real page-fault events,
project the kernel statistic once, keep that fact sticky within the epoch, and
recover only after detach, larger-epoch reset, new-object load, complete fresh
capabilities, and accepted new-epoch evidence. Keep policy-matrix tests separate
from actual hosted fault injection and performance claims.

## Scenario: Hosted Unix Datagram IPC Boundary

### 1. Scope / Trigger

- Trigger: add or change the IPC frame ABI, Unix datagram endpoint, peer
  lifecycle monitor, or an adapter that crosses the ProcBuf/Linux supervision
  boundary.
- Scope: this contract covers hosted filesystem Unix datagrams and the optional
  host-only ProcBuf/Protobuf payload adapter. It does not qualify shared memory,
  RPMsg, cryptographic identity, deployment ACL, product mechanical limits,
  PDO scaling, actual drive execution, production WCET, stress, or HIL.
- The transport must remain absent from EtherCAT, lifecycle, profile and
  ProcBuf real-time dependency trees.

### 2. Signatures

- Frame: `IpcHeader`, `IpcFrame::new`, `IpcFrame::encode_into` and
  `IpcFrame::decode`; v1 is an explicit 80-byte little-endian header with a
  4096-byte maximum payload and IEEE payload CRC-32.
- Lifecycle: `PeerPolicy`, `PeerMonitor::observe` and `PeerMonitor::status`.
- Transport: `UnixDatagramEndpoint::bind`, `send` and `receive`; focused
  verification is `make test-ipc`.
- Payloads: `ProcBufProjector::read_state_frame`, `pop_event_frame`,
  `decode_command_frame`, `admit_command_frame`,
  `prepare_procbuf_command_frame`, `prepare_motion_command_for_procbuf` and
  `AdmittedProcBufCommand::publish`; Zenoh must delegate its ProcBuf projection
  and MotionCommand target mapping to the same owner.

### 3. Contracts

- Never serialize raw Rust, ProcBuf, protobuf or C struct memory. Every offset,
  width, byte order and reserved field is part of the versioned wire contract.
- Reject zero schema/layout/robot/boot/source/sequence fields, unknown kinds,
  nonzero reserved bits, over-capacity payloads, length mismatch, CRC mismatch,
  nonempty heartbeat payloads and empty non-heartbeat payloads.
- Same-boot traffic requires strictly increasing sequence and nondecreasing
  remote time. A new boot resets those floors. Future, stale, wrong-identity and
  backwards-local-time observations do not mutate peer state.
- Endpoint calls attempt one bounded operation with no retry, sleep or worker.
  Admit only the exact configured peer path. Refuse existing bind paths and
  unlink on drop only when the path still identifies the socket created by the
  endpoint.
- IPC is transport and liveness evidence only. Command ACL, TTL, authority,
  permits and motion lifecycle decisions stay in their existing policy owners.
- State/event payloads come only from a header-validated single ProcBuf reader.
  State envelope identity, sequence and time must match the projected message;
  quality bits carry known mask in bits 0-15 and good mask in bits 16-31.
- Command frames must cross-check kind, schema, layout, numeric/text robot,
  boot, source and sequence before calling `CommandIngress`. Optional transport
  identity must match the already-cross-checked source. Pre-policy rejection
  must not mutate ingress replay, rate-limit or audit state.
- The strict target path accepts only CSP/CSV/CST raw modes, at most 32 axes,
  an in-capacity mask, exactly one zero-based finite target per selected axis,
  and non-negative velocity/torque limits. It builds authority fields only from
  the returned permit, binds robot/boot/layout/capacity, leaves unselected and
  IO slots empty, and keeps publication separately retryable.
- ProcBuf ABI v5 carries permit `policy_version`; v1-v4 attachments are rejected.
  RT consumers reconstruct permits only from a command returned by
  `ProcBuf::read_command` and still submit it to `LifecycleGuard`.

### 4. Validation & Error Matrix

- Empty nonblocking receive or local queue pressure -> `WouldBlock`; peer path
  absent/refused/reset -> `PeerUnavailable`; wrong sender ->
  `UnexpectedSource`; one-byte-over-limit receive -> `OversizedDatagram`.
- Short header, oversized payload, wrong magic/version/header size/kind,
  reserved bits, semantic zero, exact-length mismatch and CRC failure -> stable
  typed codec errors; never publish a partial frame.
- Robot/layout/source mismatch, future/stale time, replayed sequence, remote or
  local time regression -> stable typed peer errors; preserve the last accepted
  state.
- ProcBuf header/replay/lifecycle/stop/quality/non-finite/payload failures and
  command envelope/payload/target mismatches -> stable typed payload errors. Do
  not publish partial messages or invoke ingress on structural failure.
- Existing local file/socket -> `LocalPathExists`; never remove it. Other I/O
  faults retain the originating `io::Error`.

### 5. Good/Base/Bad Cases

- Good: command/state/heartbeat datagrams round-trip over two real nonblocking
  sockets; projected State/Event decode as v1 Protobuf; a validated target
  becomes an admitted ProcBuf command, RT readback reconstructs the same permit,
  and lifecycle accepts it; a same-path peer rebind with a new boot ID is
  classified Restarted.
- Base: an empty receive is `WouldBlock`, timeout boundary is still Online, and
  the next same-boot fresh sequence is Continued.
- Bad: `transmute` a ProcBuf page, duplicate projection/command mapping in each
  transport, call ingress before envelope/payload checks, retry until send
  succeeds, accept unnamed or arbitrary senders, remove a pre-existing path,
  reuse sequence in one boot, or treat CRC/path matching as authentication.

### 6. Tests Required

- Codec tests cover golden offsets/bytes, maximum payload, output capacity,
  truncation, overlength, unknown/reserved values, semantic zeros and CRC.
- Peer tests cover first contact, continued traffic, timeout boundaries,
  offline/reconnect, new boot, replay, remote/local time regression, stale and
  future frames, identity mismatch and no-mutation-on-error.
- Kernel-backed Unix tests cover both directions, command/state/heartbeat,
  `WouldBlock`, absent peer, unexpected source, oversized datagram, same-path
  restart, bind refusal and owned/replaced-path cleanup.
- Payload tests cover ProcBuf state/event projection over real sockets, quality
  mask encoding, all command identity mismatch dimensions, authenticated source,
  policy admission, every target-structure error, destination binding,
  publication retry, RT permit reconstruction, stale/non-finite/oversized state,
  and no ingress mutation on structural rejection. Existing Zenoh tests must
  continue through the shared implementation.
- Quality gates include focused Clippy/check, capability validation, full
  workspace CI and dependency-tree proof that real-time crates do not acquire
  `esop-ipc` or POSIX transport dependencies.

### 7. Wrong vs Correct

#### Wrong

Cast a process image into a datagram, block or retry under pressure, let the
transport authorize motion, infer restart from a path alone, or unlink whatever
currently occupies the endpoint path.

#### Correct

Encode one bounded versioned frame explicitly, return pressure to the caller,
validate exact source plus identity/boot/sequence/time before committing peer
state, keep safety policy outside transport, and remove only the socket inode
owned by the endpoint.

## Code Review Checklist

- Is the worst-case loop bounded by a static capacity or explicit budget?
- Are errors propagated without silently publishing partial process data?
- Are descriptor generation and cache/barrier ordering correct?
- Does the test exercise the public API and the failure path?
- Were `make ci` and the relevant cross-target checks run?
