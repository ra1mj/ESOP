# Design: Product PDO Configuration Batch

## Architecture

Keep protocol ownership in the existing controllers:

```text
StaticProductConfig + ProductMailboxBinding[]
  -> PdoConfigBatchPlan<JOBS, OPS>
  -> PdoConfigBatch<JOBS, OPS>
       owns PdoConfigController<OPS> + MailboxController

ScheduledProductionServiceScheduler
  -> current batch controller/config through existing mailbox path
  -> advance completed job
  -> release Startup only when batch phase == Complete
```

The batch layer selects immutable jobs. It does not encode CoE, create mailbox
frames, consume shared RX, or decide lifecycle readiness independently.

## Core Types

`PdoConfigJob<OPS>` owns a station address, `MailboxConfig`, and
`PdoConfigPlan<OPS>`. `PdoConfigBatchPlan<JOBS, OPS>` owns a fixed array plus a
published count; `push` validates before mutation.

`PdoConfigBatch<JOBS, OPS>` owns the frozen plan, current index, base generation,
per-job timeout values, phase/error, and the existing PDO/mailbox controllers.
Starting a batch resets all state and starts the first non-empty job. Advancing
uses at most `JOBS` iterations so empty plans cannot create an unbounded loop.
Each job generation is `base_generation.wrapping_add(index as u16)` after
validating the job count fits the generation space.

## Scheduler Binding

Extend `ScheduledPdoConfiguration` with single and batch modes while preserving
the existing constructor. Accessor methods return the active PDO controller,
mailbox controller, mailbox config, completion state, fault, and optional batch
status without duplicating scheduler branches.

At cycle entry the scheduler normalizes an already-complete current job before
checking the Startup barrier. After mailbox completion is consumed by the PDO
controller, it normalizes again. This guarantees:

- another job starts before barrier release can observe completion;
- the final job publishes batch Complete and service readiness;
- a faulted controller remains selected and no later job starts;
- the single-plan path remains unchanged.

The cycle report carries `Option<PdoConfigBatchStatus>` separately from the
single progress enum because one cycle may both complete a mailbox action and
advance the batch.

## Product Construction

`ProductMailboxBinding` binds slave position to `MailboxConfig`.
`StaticProductConfig::build_pdo_configuration_batch` validates exact one-to-one
coverage before staging jobs in product slave order. It calls the existing
`build_pdo_startup_plan` for each slave and commits the complete batch only if
all plans and bindings validate.

## Error and Restart Contract

Batch plan errors cover empty plan sets, capacity, duplicate station addresses,
and generation capacity. Product errors add missing/duplicate/unknown mailbox
positions and wrap the existing per-slave plan error with position context.
Runtime PDO/mailbox faults retain their existing precise error types. Explicit
`start` replaces plan, timing, generation and progress, clearing prior fault.

## Compatibility

The default scheduler binding remains a single controller/mailbox/config tuple.
No existing constructor arity changes. Batch memory is explicit in the caller's
const capacities, and all runtime code remains `no_std` and allocation-free.

## Testing

- Core unit tests: plan validation, ordered advance, empty jobs, generation,
  fault retention, restart.
- Scheduler tests: first completion does not release Startup; final completion
  does; report status is exact.
- Product tests: checked-in three-slave batch equality and fail-closed coverage.
- Linux simulation: two real one-write jobs pass exact mailbox readback through
  the scheduler before SAFEOP/OP resumes.

## Rollback

The feature is opt-in. Removing batch constructors and types restores the
single-plan path without changing generated schemas or wire behavior.
