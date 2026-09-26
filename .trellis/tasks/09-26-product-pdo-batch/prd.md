# Automate product PDO configuration batches

## Goal

Turn the generated per-slave PDO plans into one bounded product activation
batch that the production scheduler advances automatically while Startup is at
the PREOP configuration barrier. A product caller starts one batch, not one
controller run per slave, and Startup cannot resume until every configured job
has passed exact download/upload readback.

## Background

- `StaticProductConfig::build_pdo_startup_plan` already builds a deterministic
  fixed-capacity plan for one slave.
- `PdoConfigController` and `ScheduledPdoConfiguration` already execute one
  plan through the mailbox/DC/shared-RX path and expose exact typed faults.
- The PREOP Startup barrier currently considers the single controller Complete;
  callers must restart it for each slave before the next scheduler call.
- Product mailbox addresses remain static product input. Runtime SII discovery,
  mapping/DC descriptor generation, physical response authenticity, and HIL are
  separate requirements.

## Requirements

### R1. Fixed-capacity core batch contract

1. Add a copyable `PdoConfigJob` containing one station address, one immutable
   `PdoConfigPlan`, and its `MailboxConfig`.
2. Add a fixed-capacity batch plan with transactional push, exact length, and
   typed capacity/empty/duplicate-station validation.
3. Add a no-allocation batch controller that owns the existing PDO and mailbox
   controllers, starts once, advances jobs in frozen order, and derives a
   distinct controller generation for each job.
4. Empty per-slave plans may complete without transport, but all iteration must
   remain statically bounded by the batch capacity.

### R2. Scheduler-owned batch advancement

1. Preserve `ScheduledPdoConfiguration::new` and all existing single-plan call
   sites.
2. Add an opt-in batch binding whose active controller, mailbox controller, and
   mailbox configuration are selected from the current job.
3. Before Startup barrier release, and after a PDO transaction reaches Complete,
   advance the batch to the next job. A completed current controller shall not
   release Startup while another job remains.
4. Missing, idle, active, retrying, or faulted batch work shall keep the barrier
   closed. A fault remains on its exact job until explicit batch restart.
5. Production reports shall expose bounded batch status: phase, current index,
   total jobs, and current station address.

### R3. Product batch construction

1. Add a product mailbox binding keyed by configured slave position.
2. `StaticProductConfig` shall build one batch job for every configured slave in
   frozen product order using the existing per-slave plan builder.
3. Batch construction shall reject missing, duplicate, or unknown mailbox
   bindings, duplicate station addresses, insufficient job/operation capacity,
   and any existing per-slave plan error before returning a partial batch.
4. The checked-in three-slave product shall build a three-job batch whose
   station addresses, mailbox bindings, and plan bytes match the individual
   generated plans.

### R4. Compatibility and fail-closed behavior

1. Preserve `no_std`, allocation-free, fixed-capacity, non-blocking execution.
2. Do not duplicate CoE, mailbox, PDO mapping, request-pool, or scheduler
   transport logic.
3. Existing single-plan scheduler behavior and no-batch Startup behavior shall
   remain source-compatible.
4. Batch restart shall clear prior progress/fault and use only the newly supplied
   immutable plan and timing values.

### R5. Qualification boundary

1. Software tests prove deterministic ordering, request ownership, readback,
   failure retention, and PREOP release gating only.
2. This task does not discover MailboxConfig, generate mapping/DC jobs, perform
   AL error acknowledgement, qualify full-period WKC, or claim physical/HIL
   authenticity, target WCET, or functional safety.

## Acceptance Criteria

- [x] A two-job batch automatically starts job 2 only after job 1 completes and
      reports the correct index/station/generation without caller restart.
- [x] Startup remains at `AwaitingConfiguration` after the first job and resumes
      only after every batch job is Complete.
- [x] A fault or timeout on job N preserves job N, blocks later jobs and Startup,
      and requires explicit batch restart.
- [x] Empty jobs are skipped in bounded order and an all-empty batch reaches
      Complete without allocating a request.
- [x] The checked-in three-slave generated product builds exact drive/drive/IO
      jobs from position-keyed mailbox bindings and rejects every coverage or
      capacity mismatch transactionally.
- [x] Existing single-plan scheduler, PDO controller, Startup, lifecycle, and
      generated-product tests remain green.
- [x] Core/product-config `aarch64-unknown-none`, Clippy, full CI, BPF, capability
      manifest, and Zenoh gates pass.
- [x] README, product configuration docs, EtherCAT requirements, software PRD,
      capability claims, and Trellis specs describe automatic PDO batches while
      retaining mapping/DC discovery, HIL, WCET, and authenticity limits.

## Out of Scope

- Automatic MappingConfigController or DcController job batches.
- Runtime SII/ESI mailbox, SM, FMMU, or DC descriptor discovery.
- Dynamic product schemas, heap-backed variable job lists, or parallel jobs.
- Physical slave interoperability, long-duration soak, target timing, and
  certification evidence.
