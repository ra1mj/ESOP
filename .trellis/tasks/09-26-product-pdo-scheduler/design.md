# Design: Production-Scheduled PDO Configuration

## Architecture Boundary

The integration composes the two existing state machines instead of merging
them:

```text
PdoConfigController
  -> PdoConfigAction (raw CoE SDO bytes)
  -> MailboxController::start(CoE, bytes)
  -> MailboxAction / ControlRequestPool
  -> ScheduledDomainBank::run_dc_and_mailbox_cycle
  -> MailboxController::response()
  -> PdoConfigController::accept(response)
```

`PdoConfigController` remains the owner of plan order, download/upload steps,
and exact readback comparison. `MailboxController` remains the owner of mailbox
framing, counters, register access, polling, retries, and response extraction.
The scheduler owns the cross-cycle association and the single request handle.

## Public Binding

Add an ephemeral binding carried by `ScheduledProductionServices`:

```text
ScheduledPdoConfiguration<'a, OPS> {
    controller: &'a mut PdoConfigController<OPS>,
    mailbox: &'a mut MailboxController,
    mailbox_config: MailboxConfig,
}
```

The binding prevents independent pieces from being supplied without the
configuration that joins them. It is recreated by the caller for each cycle;
the referenced controllers retain their state. The same mailbox controller can
later be reused for normal mailbox work, preserving its rolling mailbox counter.

Append `PDO_OPS` as a defaulted const generic on `ScheduledProductionServices`
so existing three-const call sites keep compiling. Keep the existing `new`
constructor and add a consuming `with_pdo_configuration` builder for the new
binding.

## Scheduler State

Add `PdoConfiguration` to the service kind and store one
`Option<PdoConfigAction>` in `ScheduledProductionServiceScheduler`. This action
is immutable evidence connecting the PDO FSM to the mailbox FSM across cycles.

Selection order becomes:

```text
Startup -> PdoConfiguration -> Mapping -> DcConfiguration -> Mailbox
```

An existing request or still-active selected service keeps ownership. The
ordering assumes Startup was targeted to PREOP when PDO configuration is
required; the scheduler does not synthesize a complete AL activation sequence.

## Transaction Preparation

When PDO Configuration is selected and has no bound action:

1. Call `PdoConfigController::next_action(now_ns)`.
2. Reject an action whose deadline is already reached through the PDO timeout
   path without acquiring a control slot.
3. Clamp a copy of `MailboxConfig::timeout_ns` to the action's remaining
   absolute budget.
4. Start the supplied mailbox controller with CoE protocol and exact action
   identity/payload.
5. Store the PDO action and let the existing mailbox enqueue helper acquire a
   request only when the mailbox action is due.

The mailbox controller may be Sending, CheckingStatus, Polling, Complete, or
Faulted while one PDO action remains pending. Poll and retry delays yield no
request, preserving pool capacity.

## Request and Action Validation

For PDO Configuration, `ensure_request_matches` validates both layers:

- scheduler-bound PDO action equals `PdoConfigController::pending()`;
- the request-pool entry matches `MailboxController::pending()` in datagram
  index, generation, address, operation, lengths, deadline, and padded payload.

The existing mailbox cycle remains the only component allowed to consume and
release a terminal control request. No PDO response is accepted directly from
the request pool.

## Completion and Failure

After `run_dc_and_mailbox_cycle`:

- Mailbox transport progress is reported while the transaction is still
  sending, polling, waiting, or retrying.
- On mailbox `Complete`, copy the response payload, call PDO `accept` with the
  stored action/generation, clear the association, and report PDO progress.
- On terminal mailbox error, call a new action-checked PDO transport-failure
  method. The PDO controller latches `PdoConfigError::Mailbox(error)`, clears
  pending state, and remains Faulted, blocking lower-priority services.
- A response mismatch remains the existing precise PDO readback error and does
  not advance `operation_index`.

The scheduler must clear the stored association only after PDO acceptance or a
latched transport fault. A cycle-level preflight error keeps immutable state
available for a caller retry unless the existing prepared-request cleanup rule
applies.

## Progress and Lifecycle Contract

Add a small production-facing progress enum that distinguishes mailbox
transport progress from PDO plan progress. Add PDO variants to production
progress/fault enums and map the service to the Configuration/CoE lifecycle
gate. `service_ready` is true only when the PDO controller is Complete and the
transport cycle has no failure.

The underlying transport remains a `ScheduledMailboxCycleReport`, so existing
Bank confirmation, receive-quality accessors, post-RX deadline evidence, and
stable production owner integration are reused unchanged.

## Compatibility

- Existing callers use `ScheduledProductionServices::new` unchanged.
- The new defaulted const generic is zero for callers without PDO service.
- Core service reports gain additive enum variants; exhaustive downstream
  matches in lifecycle code are updated explicitly.
- `PdoConfigController` gains only a typed transport-failure entry point and a
  mailbox error variant; direct caller-driven protocol tests remain valid.
- No generated artifact or product schema changes.

## Testing Strategy

Core tests cover the action-checked transport-failure API. Linux simulated-port
integration drives protocol-correct EtherCAT mailbox responses and proves the
full download/readback sequence, fixed priority, cross-generation waiting,
Prepared rebuild, retry/no-slot behavior, mismatch, timeout, fault blocking,
explicit restart, report confirmation, and lifecycle projection. Existing
production scheduler tests remain unchanged where the default PDO capacity is
zero.

## Rollout and Rollback

The feature is opt-in through `with_pdo_configuration`; deployments not
supplying the binding retain current behavior. Rollback removes the additive
binding, enum variants, scheduler association, and PDO transport-failure API.
There is no persistent-state or schema migration.

## Qualification Boundary

Passing tests proves bounded software ownership and exact response routing
through the simulated production mailbox path. It does not prove that the
caller opened the correct PREOP window, that a physical response is authentic,
or that a drive, network, target CPU, brakes, or mechanics satisfy product or
functional-safety requirements.
