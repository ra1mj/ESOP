# EtherCAT Explicit Rescan

## Goal

Deliver the second independently verifiable slice of `REC-001`: an explicit,
bounded asynchronous `rescan` operation that invalidates stale EtherCAT
topology/configuration evidence before its first wire action, reuses the
existing Startup scan/SII/topology protocol authorities, coexists with cyclic
Domain/DC traffic, and stops at verified PREOP rather than returning slaves to
OP implicitly.

## Functional Requirements

1. Add stable fixed-size rescan handles, phase, status, progress, result, and
   error value types suitable for diagnostics in `no_std` code.
2. Expose a named `StartupController::start_rescan(generation, now_ns,
   deadline_ns)` operation plus immutable status/result lookup. Submission is
   valid only from a completed or faulted retained Startup plan, with a future
   absolute deadline and no currently active Startup operation.
3. Reuse the retained expected-slave list, product profiles, station-address
   base, scan/identity/SII/request timeouts, and every existing scan, identity,
   Requesting ID, mailbox, FMMU/SyncManager discovery, SII configuration, DC
   topology, AL, and OpOnly authority. Do not duplicate their wire protocols.
4. Before `start_rescan` returns success, clear the slave table, scan records,
   verified Requesting ID/mailbox/FMMU/SyncManager/SII/DC evidence, selected
   reference clock, DC topology, configuration barrier state, AL fault record,
   and readiness-bearing Startup phase.
5. Run the retained Startup plan with target state PREOP and no external
   configuration-service barrier. Rescan success may publish newly verified
   topology/configuration evidence but may not request SAFEOP or OP, rebuild
   the process image, reconfigure PDO/DC registers, or refresh motion commands.
6. Apply the caller's absolute deadline to the whole reused Startup pipeline.
   Every nested operation and wire request must use the lesser of its existing
   configured timeout and the remaining rescan budget.
7. Add a distinct production `Rescan` service projection at Startup priority.
   It must preserve an already in-flight service request, use the shared
   control pool, retain accepted requests across cycles, and expose named
   progress/fault/readiness evidence.
8. Project active or faulted rescan work into the topology lifecycle gate.
   Readiness is revoked synchronously at submission and can be restored only
   after the existing full Startup verification reaches Ready at PREOP; all
   independent drive, DC, domain, command-age, and permit gates still apply.
9. Latch the first Startup/protocol failure into the rescan result. A new
   rescan may be submitted explicitly after terminal failure, but no timeout,
   WKC/link/protocol fault may trigger an automatic retry or automatic rescan.
10. Keep the capability boundary partial: `request_state` and `rescan` become
    implemented, while `reconfigure_slave`, the unified recovery facade, mixed
    recovery-load qualification, target timing, HIL, and conformance remain
    planned.

## Determinism and Safety Requirements

- No heap allocation, blocking, sleeping, recursion, unbounded polling,
  background recovery thread, or automatic retry.
- Submission validation must complete before retained readiness is cleared.
  Invalid or busy submissions cannot erase the previous verified evidence.
- At most one rescan control datagram is admitted per production cycle.
  Accepted in-flight work is retained and never retransmitted merely because
  its response arrives in a later cycle.
- Exact operation, index, generation, address, payload, response length,
  deadline, WKC policy, and terminal control-pool matching remain owned by the
  reused Startup and shared-control contracts.
- A rescan result is communication/configuration evidence only. It does not
  prove physical response provenance, safe outputs, STO, target WCET/jitter,
  HIL, ETG conformance, or functional-safety qualification.

## Acceptance Criteria

- [x] Public rescan types and `StartupController` APIs are fixed-size,
      deterministic, `no_std`, handle-stable, and re-exported from
      `esop-ethercat-core`.
- [x] Unit tests prove transactional submission, stale handles, explicit retry
      after failure, whole-operation deadline capping, synchronous evidence
      invalidation, PREOP-only completion, and first-fault retention.
- [x] Startup tests prove retained expected/profile reuse and replacement of
      scan, slave, SII/register, mailbox, DC topology/reference-clock, AL, and
      configuration evidence before the first rescan action.
- [x] Production scheduler tests prove distinct Rescan selection, Startup-level
      priority, no in-flight preemption, exact request matching, terminal
      consumption, persistent fault selection, and no automatic fault trigger.
- [x] Linux simulation proves cyclic `LRW` and DC `FRMW` precede rescan `APRD`,
      a delayed probe response spans cycles without retransmission, one shared
      control slot remains sufficient, and topology readiness stays false until
      terminal verified success.
- [x] Existing startup, state-request, PDO/configuration, mailbox, register
      request, lifecycle, generated-product, and no_std behavior remains
      compatible.
- [x] Focused tests, `make test-hil`, `make ci`, `git diff --check`, and exact
      GitHub Actions for the pushed commit succeed.

## Out of Scope

- Single-slave `reconfigure_slave` and the final unified recovery facade.
- Hot-plug process-image resizing or accepting a topology different from the
  retained fixed product plan.
- Automatic retry, policy-driven recovery, automatic SAFEOP/OP return, or
  automatic state requests after rescan.
- Real-slave interoperability, hardware timing qualification, HIL, ETG
  conformance, safe-output certification, STO control, or functional safety.
