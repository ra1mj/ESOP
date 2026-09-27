# Automatic slave-copy cycle-owner integration

## Goal

Apply the immutable slave-to-slave copy plans owned by an activated product
automatically in the stable production cycle. A verified source receive from
cycle N must update one stable, published process image for the due outputs of
cycle N+1 before any auxiliary frame is built.

## Background

The previous product-slave-copy task generated and activated a validated
`SlaveCopyPlanSet`, but execution remained a manual test-only operation.
`ScheduledProductionCycleOwner` already owns the stable RX/output handoff, and
`ScheduledAuxiliaryOutputs` already submits the next generation. The missing
boundary is a fixed-capacity process-image publication step between those two
operations.

## Requirements

### R1. Fixed-capacity publication

- Add an allocation-free double-page process image whose published page is
  immutable while the inactive page is prepared.
- Preparing cycle N+1 starts from the last published bytes so unrelated product
  outputs are retained.
- The publication cycle is observable and must advance monotonically.

### R2. Transactional copy execution

- `ScheduledDomainBank` shall execute only plans whose target Domain is due for
  the requested target cycle.
- Every due plan must be preflighted before the first target byte changes.
- Missing source/target Domain bindings, wrong cycle order, address/size
  mismatch, or another copy error shall publish nothing and preserve the old
  page byte-for-byte.
- Valid, stale, and invalid-source outcomes shall preserve the existing
  `SlaveCopyPlan` value/quality semantics and be returned in plan order.

### R3. Stable-cycle ownership

- A copy-enabled `ScheduledProductionCycleOwner` shall bind one immutable plan
  set at activation and reject plans targeting the lifecycle-owned motion
  Domain.
- Completing a mailbox, control, production-service, or raw receive cycle shall
  validate the existing handoff, publish copies for the next cycle, and only
  then enter `OutputPending`.
- A publication failure shall enter a terminal fail-closed owner phase; output
  settlement and receive replay must be rejected until the owner is rebuilt.
- Release evidence shall report the number of due copies applied.

### R4. Long-lived TX binding

- `ScheduledAuxiliaryOutputs` shall support auxiliary plans that read from the
  shared published process image instead of a frozen per-Domain byte slice.
- Existing static-image construction shall remain source compatible.
- Shared-image plan bounds shall be checked before TX; a shared-image output
  path without the required cycle-owned publication shall fail before sending.

### R5. Product integration and compatibility

- The generated simulator product's activated plan set shall drive the IO
  mirror and quality byte through the copy-enabled production owner.
- Products with an empty plan set and existing static auxiliary-output callers
  shall preserve current behavior.
- The cyclic path shall remain `no_std`, allocation-free, lock-free, bounded by
  const capacities, and free of logging or sleeping.

## Acceptance Criteria

- [x] A fresh drive position received in cycle N appears in the IO frame armed
  for cycle N+1 with quality `1`, without a manual `SlaveCopyPlan::apply_*` call.
- [x] A bad-WKC or stale source publishes configured fallback bytes and quality
  `0` on the next due IO cycle.
- [x] A non-due target is not copied or transmitted early.
- [x] A copy preflight failure leaves the previously published process image
  unchanged, sends no new output, and faults the production owner.
- [x] Replayed receive completion, skipped copy publication, wrong publication
  cycle, and motion-Domain copy targets are rejected.
- [x] The release report carries the exact number of due copies applied.
- [x] Existing stable-cycle, static auxiliary-output, generated-product, and
  empty-copy tests continue to pass.
- [ ] Focused tests, `no_std` target checks, Clippy with warnings denied,
  `make ci`, and the exact pushed GitHub Actions run all pass.

## Out of Scope

- Physical slave response provenance, target-hardware WCET, long-duration
  qualification, real-drive/IO HIL, FSoE/STO, or functional-safety claims.
- Automatic copies into the lifecycle-owned motion Domain. Those outputs need a
  separate product policy proving compatibility with CiA 402 safety handling.
- Concurrent multi-thread readers of unpublished process-image pages.
