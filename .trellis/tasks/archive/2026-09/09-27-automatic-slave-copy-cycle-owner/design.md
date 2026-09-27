# Design

## Data flow

```text
ActivatedProduct::slave_copy_plans()
  -> ScheduledProductionCycleOwner::with_slave_copies(...)
  -> complete_* validates cycle N shared RX
  -> ScheduledDomainBank preflights all copies due for N+1
  -> inactive SlaveCopyProcessImage page is updated
  -> one atomic owner-side page publication
  -> StopCycleContext builds auxiliary frames from published shared image
  -> settle_output validates N+1 handoff and reports copy count
```

## Core process image

`SlaveCopyProcessImage<const BYTES: usize>` owns two fixed byte arrays, the
published page index, and the last published target cycle. Its public API only
exposes the published page and metadata. Staging, mutation, and commit remain
crate-owned so a caller cannot publish a partially applied page.

`SlaveCopyPlan` retains its local source/target field offsets and additionally
records the target Domain's product process-image base offset. Existing
`apply_to_domains` and `apply_within_domain` keep local-image behavior. The new
scheduled publication path reads a source Domain's committed local image and
writes the target field at `domain_base + local_offset` in the global product
image.

`ScheduledDomainRx` remains sealed but exposes read-only committed input bytes
and quality. Scheduled frame binding accepts exact input-bearing `LRD`/`LRW`
segments and bounded pure `LWR` output segments, so generated split input/output
plans bind without weakening logical-address or local-image bounds.
`ScheduledDomainBank::publish_slave_copies` checks that RX is
finished, `target_cycle == last_cycle + 1`, and every referenced Domain is
bound. It performs a complete first pass for address, bounds, due-cycle, and
quality classification, then copies the active page to staging, writes all due
plans in a second pass, and commits once. The report stores fixed-capacity
`(plan_index, SlaveCopyOutcome)` records in original plan order.

## Production owner

Add a defaulted copy-plan capacity parameter to
`ScheduledProductionCycleOwner`. The existing `new` constructor creates the
zero-plan compatibility form. `with_slave_copies` binds a plan set and checks
that every target is an auxiliary shared-image Domain, never the motion
Domain.

Copy-enabled completion methods take the caller-owned
`SlaveCopyProcessImage`, validate the existing receive evidence, publish the
next target cycle, then transition to `OutputPending`. Any publication error
transitions to a new `Faulted` phase. The old no-copy completion methods remain
available only on the zero-plan owner, preventing accidental bypass for a
copy-enabled owner.

The output-pending state retains the applied-copy count. `settle_output`
projects it into `ScheduledProductionRelease`; task release otherwise keeps
the existing process/deadline/State/event conditions.

## Shared auxiliary image

Keep `AuxiliaryOutputEntry` and `ScheduledAuxiliaryOutputs::new` unchanged for
static callers. Add a shared-image entry and constructor whose internal image
source is a marker rather than a borrowed slice. The constructor validates the
schedule, frame plans, indices, write overlap, target ownership, and declared
global image length.

Bank-aware `StopCycleContext` entry points require a typed
`SlaveCopyPublishedImage` snapshot and pass the same backing bytes to auxiliary
submission. The boundary verifies target cycle, exact length, and pointer
identity before any frame acquisition or TX. The legacy entry that only
receives a detached quality array rejects shared-image outputs because it
cannot prove the cycle publication. Frame construction continues to use the
existing immutable `FramePlanSet` and master ownership path.

## Compatibility and failure behavior

- No heap, locks, dynamic dispatch additions outside the already sealed Domain
  bank, or cyclic text diagnostics.
- Existing static output objects do not require a process-image publisher.
- Empty copy sets publish a stable page with zero outcomes and retain behavior.
- Preflight errors do not swap pages. Owner faults prevent the already-consumed
  RX from being misrepresented as still armed or from being replayed.
- Accepted prior TX ownership is unchanged; this task only prepares frames for
  the next generation.

## Evidence boundary

Simulation proves ordering, exact bytes, stale/WKC fallback, page atomicity,
and fail-closed owner state. It does not prove physical output execution,
timing bounds, real-device interoperability, or functional safety.
