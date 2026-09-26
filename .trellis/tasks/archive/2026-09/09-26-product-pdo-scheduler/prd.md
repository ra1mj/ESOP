# Integrate PDO Configuration into Production Scheduler

## Goal

Run each generated PDO assignment/mapping SDO transaction through the existing
bounded production service scheduler and EtherCAT mailbox transport, so exact
readback verification participates in the same request ownership, deadline,
recovery, and lifecycle configuration gate as the other production services.

## Background

- `PdoConfigController` owns the ordered CoE SDO download/upload verification
  state machine, but its actions currently stop at a raw CoE payload.
- `MailboxController` owns EtherCAT mailbox framing, fixed-address register
  access, polling, mailbox counters, retries, and response extraction.
- `ScheduledProductionServiceScheduler` already arbitrates one production
  service request across Startup, Mapping, DC Configuration, and Mailbox while
  preserving `Prepared`, `InFlight`, and terminal request semantics.
- A raw `PdoConfigAction` cannot be inserted directly into
  `ControlRequestPool`: it does not contain an EtherCAT datagram index,
  register address, operation, or mailbox header.
- Product configuration currently supplies the PDO plan and station address;
  mailbox register addresses/capacities remain runtime/discovery inputs.

## Requirements

### R1. Fixed-priority service selection

1. Add a distinct PDO Configuration production service kind.
2. When several services are active and no request is already owned, use the
   fixed order `Startup -> PDO Configuration -> Mapping -> DC Configuration ->
   Mailbox`.
3. An already selected active service or owned request shall not be preempted.
4. A faulted PDO Configuration service shall retain priority and block lower
   services until the caller explicitly restarts its controller.

### R2. Mailbox transport binding

1. The caller shall provide one mutable `PdoConfigController`, one mutable
   `MailboxController`, and the matching `MailboxConfig` as one scheduled PDO
   service binding.
2. The scheduler shall start the mailbox controller with `MailboxProtocol::CoE`,
   the PDO action's station address/generation, and the exact SDO request bytes.
3. The existing mailbox controller and `run_dc_and_mailbox_cycle` path shall
   remain the only owners of mailbox headers, counters, register operations,
   polling, retry policy, control-request framing, and shared RX completion.
4. The mailbox configuration timeout shall not extend a PDO action beyond its
   absolute deadline.

### R3. End-to-end action identity

1. At most one `PdoConfigAction` may be bound to the scheduled mailbox
   transaction at a time.
2. Across cycles, the scheduler shall verify that the stored PDO action still
   equals the controller's pending action and that the request-pool entry still
   matches the mailbox controller's pending action.
3. A mailbox response may be passed to `PdoConfigController::accept` only after
   the matching mailbox controller reaches `Complete` and exposes a CoE
   response payload.
4. Modified, stale, missing, or cross-controller bindings shall fail closed
   before they can advance the PDO plan.

### R4. Request ownership and recovery

1. Preserve existing production-service semantics: `Prepared` is sent at most
   once, `InFlight` may cross production generations without retransmission,
   and terminal requests are consumed and released by the matching mailbox
   state machine.
2. A `Prepared` request that cannot be sent because of DC/deadline rejection
   shall be released and rebuilt from the unchanged mailbox/PDO action.
3. Mailbox polling delay and retry delay shall not allocate a request early.
4. Mailbox retries shall preserve the current PDO operation/step. Terminal
   mailbox errors shall become a typed PDO transport fault and leave the PDO
   operation index unchanged.

### R5. Reporting and lifecycle projection

1. Production service reports shall identify PDO Configuration separately and
   expose either mailbox transport progress or PDO controller progress.
2. The exact PDO or mailbox-derived fault, scheduler recovery state, surviving
   request handle, and readiness shall remain observable.
3. PDO Configuration is a Configuration/CoE lifecycle gate. It is ready only
   after the PDO controller reaches `Complete`; transport success for one SDO
   is not sufficient.
4. Existing Bank report confirmation and stable production-cycle ownership
   shall continue to work without caller-supplied readiness overrides.

### R6. Compatibility and bounded execution

1. Existing callers that do not schedule PDO Configuration shall retain their
   current constructor shape and behavior.
2. The implementation shall remain `no_std`, allocation-free, fixed-capacity,
   and bounded by existing plan, mailbox, request-pool, and Domain capacities.
3. No new thread, blocking wait, dynamic dispatch, or duplicate mailbox
   protocol implementation may be introduced.
4. Public fallible paths shall use fixed-size typed errors.

### R7. Activation boundary

1. This task assumes the caller starts PDO configuration only while the slave
   is in a mailbox-capable configuration state, normally PREOP.
2. Scheduler integration shall not claim to orchestrate the complete
   `INIT -> PREOP -> configuration -> SAFEOP -> OP` product activation sequence.
3. Software simulation shall not be described as physical readback, device
   interoperability, timing qualification, or functional-safety evidence.

## Acceptance Criteria

- [x] A single generated-style PDO write traverses SDO download, mailbox send
      and poll, exact CoE response delivery, upload verification, and completion
      through `ScheduledProductionServiceScheduler`.
- [x] The scheduler selects `Startup -> PDO Configuration -> Mapping -> DC
      Configuration -> Mailbox` and never preempts an owned/in-flight request.
- [x] A PDO mailbox request may complete under a later production generation
      without retransmission or relaxing Domain/DC generation checks.
- [x] DC/TX rejection releases only an unsent `Prepared` request and reports
      `RebuildRequest`; the same immutable PDO and mailbox actions are rebuilt.
- [x] Poll/retry waits do not hold a control slot, mailbox retry preserves the
      PDO operation, and terminal transport failure faults PDO Configuration.
- [x] Exact readback mismatch, transport timeout, action/request mismatch, and
      missing PDO/mailbox controller bindings fail closed and block lower
      priority services.
- [x] The final PDO transaction reports ready only after the complete plan is
      verified; lifecycle projection clears `coe_ready` while incomplete or
      faulted and preserves report confirmation.
- [x] Existing production-service call sites compile without PDO integration,
      and all existing scheduler/lifecycle behavior remains covered.
- [x] Core, Linux simulated-port, product-config, `no_std`, Clippy, full CI,
      BPF, capability-manifest, and Zenoh checks pass.
- [x] PRD, product configuration, lifecycle, robotics plan, capability claims,
      and Trellis guidance state the delivered scheduler/mailbox evidence and
      retain the PREOP orchestration and physical HIL boundary.

## Out of Scope

- Full multi-slave product activation orchestration or automatic AL state
  transitions around the configuration window.
- Generating/discovering `MailboxConfig` from ESI/SII or selecting one of
  multiple mailbox channels.
- Parallel PDO configuration of several slaves or more than one production
  service request per cycle.
- Complete Access, SDO Information, dynamic object discovery, vendor-specific
  object sequences, or runtime ESI parsing.
- Physical slave interoperability, HIL, measured WCET/jitter, mechanics,
  braking, STO/FSoE, or functional-safety qualification.
