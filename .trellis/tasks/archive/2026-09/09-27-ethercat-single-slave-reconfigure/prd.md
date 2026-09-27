# EtherCAT Single-Slave Reconfiguration

## Goal

Add an explicit, bounded `reconfigure_slave(position, deadline)` operation that
reconfigures exactly one retained and verified EtherCAT slave, preserves every
unrelated slave and the cyclic Domain/DC path, stops in verified PREOP, and
publishes fixed-capacity typed progress and terminal evidence.

## Background

- `docs/ethercat-master-requirements.md` `REC-001` requires an explicit
  single-slave reconfiguration operation and evidence that recovery work does
  not break cyclic traffic.
- Startup already retains the authoritative slave position, station address,
  identity, mailbox/SII, SyncManager/FMMU register banks, AL state, DC
  topology, and selected reference clock.
- PDO, SM/FMMU mapping, watchdog, DC clock, DC Sync, AL transition, and
  OpOnly-SyncManager controllers already own their respective wire protocols.
  Reconfiguration must compose those authorities rather than duplicate them.
- The production scheduler always services cyclic Domain/DC work first and
  retains one accepted service request across cycles without retransmission.

## Requirements

### R1. Explicit operation contract

Expose one fixed-capacity asynchronous operation with a stable nonzero handle,
immutable status/progress snapshots, a typed terminal result, and typed
submission/runtime errors. Only one single-slave reconfiguration may be active
at a time. Polling a stale handle must be distinguishable from polling the
current operation.

### R2. Transactional admission

Submission is accepted only while startup evidence is Ready, the requested
position resolves to one online and configured slave, retained identity and
station evidence are internally consistent, all required per-slave plans fit
their fixed capacities, the absolute deadline is in the future, and no prior
operation is active. Every validation occurs before retained readiness is
mutated or a wire action becomes available.

### R3. Target-only evidence revocation

After admission and before the first wire action, mark only the target slave as
not configured. Preserve unrelated slave records, identities, observed states,
mailbox/SII/register evidence, topology, and generated plans byte-for-byte.
Lifecycle topology readiness must fail closed while the operation is active or
faulted.

### R4. Safe transition to PREOP

If the target exposes OpOnly SyncManagers, disable and verify them before
requesting a lower AL state. Reuse the existing AL transition authority to
reach verified PREOP. The operation must not request SAFEOP or OP and must not
re-enable OpOnly SyncManagers during this operation.

### R5. Targeted configuration sequence

After verified PREOP, execute the target's configuration in this order:

1. PDO mailbox configuration when present;
2. watchdog configuration when present;
3. SyncManager/FMMU mapping clear, write, and exact readback;
4. DC clock configuration when required;
5. DC Sync configuration when required.

Every write must address the target station only. A DC Sync phase may read the
retained reference clock station, including when it is another slave, but must
not write that station or any other unrelated slave.

### R6. Retained plan authority

Build the operation plan from already verified startup and product evidence:
target station/identity, mailbox configuration, per-slave PDO plan, caller-
frozen mapping table, optional watchdog entry, and validated DC topology/
timing context. Plan construction is bounded and performs no runtime heap
allocation.

### R7. Whole-operation deadline and fault retention

One absolute operation deadline caps every phase, nested request timeout, and
mailbox transaction. The first timeout, WKC mismatch, malformed response, AL
error, generation mismatch, unavailable capacity, retained-evidence conflict,
or child-controller error terminates the operation. Preserve the first fault,
leave the target unconfigured, perform no automatic retry, and require a new
explicit recovery request after the caller restores trustworthy context.

### R8. Scheduler coexistence

Add a named production service for reconfiguration after Startup and Rescan but
before ordinary configuration controllers, StateRequest, Mailbox, and ad-hoc
register requests. Cyclic Domain/DC processing remains first. Admit at most one
bounded service action per cycle, retain accepted in-flight ownership across
cycles, and never duplicate a request because its response is delayed.

### R9. Verified completion

Success requires every applicable child controller to complete exact readback
and the target to remain observed in PREOP. Commit only the target record back
to configured. Completion is communication/configuration evidence only: it
must not restore OP, publish fresh motion commands, clear unrelated lifecycle
gates, or imply hardware timing or functional-safety qualification.

### R10. Compatibility and boundedness

Keep existing startup, PDO, mailbox, register-request, state-request, rescan,
and production APIs compatible through additive changes. The cyclic path must
remain `no_std`, allocation-free, recursion-free, and bounded by compile-time
capacities.

## Acceptance Criteria

- [x] Invalid position, stale/inconsistent evidence, expired deadline,
      over-capacity plan, and busy submissions are rejected transactionally;
      no record changes and no action is exposed.
- [x] Accepted submission invalidates only the target's configured flag before
      its first wire action; unrelated retained records and evidence remain
      unchanged.
- [x] Tests cover stable/current/stale handles, phase snapshots, terminal
      result retention, and first-fault retention.
- [x] An OpOnly target is disabled and verified before the PREOP AL request;
      the operation never requests SAFEOP/OP and never implicitly re-enables
      OpOnly SyncManagers.
- [x] Focused tests prove PDO, optional watchdog, mapping, targeted DC clock,
      and targeted DC Sync execute in order with exact readback and only target
      writes. A non-target DC reference may be read but is never written.
- [x] The absolute operation deadline caps nested controller/request deadlines,
      and timeout/fault leaves the target unconfigured without retry.
- [x] Production scheduler tests prove Domain/DC-first ordering, one bounded
      control slot, reconfiguration priority, retained in-flight ownership,
      and no duplicate transmission across delayed responses.
- [x] Linux simulation runs cyclic LRW/FRMW traffic concurrently with a
      multi-cycle reconfiguration and proves the unrelated slave remains
      unchanged while the target finishes configured in PREOP.
- [x] Lifecycle tests prove active/faulted reconfiguration closes topology
      readiness and successful PREOP completion does not restore OP or bypass
      health, DC, command-age, or CiA 402 gates.
- [x] Focused core/product/lifecycle/Linux tests, `make test-hil`, `make ci`,
      and exact GitHub Actions for the delivered commit succeed.

## Out of Scope

- Topology rescan, hot-plug discovery, or process-image resizing.
- Multi-slave or broadcast reconfiguration.
- Automatic retry, policy orchestration, command refresh, or automatic return
  to SAFEOP/OP.
- Dynamic product configuration outside the compiled fixed-capacity contract.
- Real-hardware interoperability, target WCET/jitter qualification, HIL, ETG
  conformance, STO/FSoE behavior, or functional-safety claims.
