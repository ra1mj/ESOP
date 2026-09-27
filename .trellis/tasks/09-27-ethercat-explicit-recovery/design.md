# Design: EtherCAT Explicit Recovery Control Plane

## Architecture

Recovery is an additive control plane around the existing production runtime:

```text
application/operator request
          |
          v
 fixed-capacity recovery controller
          |
          +--> AL transition authority (`request_state`)
          +--> startup scan/SII/topology authority (`rescan`)
          +--> SM/FMMU/PDO/DC authorities (`reconfigure_slave`)
          |
          v
 production scheduler control budget
          |
          +--> cyclic Domain/DC path always runs first
          +--> at most one bounded recovery action is admitted
          |
          v
 typed progress/result + lifecycle/diagnostic evidence
```

Each child task owns one independently testable controller or integration
slice. The final child unifies the public recovery status surface and closes
cross-operation documentation and budget evidence.

## Shared Operation Contract

Every operation has an explicit submission boundary, absolute deadline,
generation, progress phase, and terminal result. Submission validates all
caller-owned data before mutating controller state. The cyclic driver advances
only the current bounded action and retains in-flight ownership across cycles;
it never retransmits an accepted action merely because the response arrives in
a later cycle.

Only one operation of a given controller may be active initially. This keeps
RAM and scheduling cost fixed and makes busy/backpressure explicit. A later
fixed-capacity FIFO is allowed only if a child demonstrates a real product
need and preserves stable handles and deterministic ordering.

## Retained Evidence

Startup/topology state remains the authority for slave position, station
address, identity, online/configured flags, and observed AL state. Recovery
controllers consume immutable request context from that authority and commit a
new observation only after exact operation/index/generation/address/deadline/
WKC validation succeeds. Stale or contradictory evidence ends the operation;
it is never patched optimistically.

## Scheduler and Lifecycle Ordering

Normal production cycles continue to submit/receive Domain and DC traffic
before considering service work. Recovery becomes another bounded production
service using the shared control pool. A recovery operation that can make the
current configuration unsafe revokes topology/configuration readiness before
its first wire action. The lifecycle therefore suppresses motion publication
even if the recovery controller has not yet received its response.

Terminal communication success does not directly restore motion readiness.
Only the existing startup/configuration, health, DC, command freshness, and
CiA 402 gates can publish a motion-capable lifecycle state.

## Child Boundaries

### Runtime State Requests

Wrap the existing `AlTransitionController` with an application-facing bounded
request lifecycle. Resolve position to verified station/current-state/profile
context, advance through the existing AL action/completion API, reconcile the
observed state, and expose immutable status/results. Integrate it into the
production scheduler without changing the PDO path.

### Explicit Rescan

Add a named rescan operation that deliberately invalidates stale retained
topology/configuration evidence, then drives the existing scan, station
assignment, SII, topology validation, and startup barriers. It is not a hidden
response to WKC/link errors.

### Single-Slave Reconfiguration

Build a targeted plan from already verified product/topology evidence and
reuse the existing SM/FMMU/PDO/DC controllers. Other slave records and plans
remain untouched. Reconfiguration ends before automatic OP entry.

### Integration and Qualification

Unify snapshots/errors, emit diagnostic events, run mixed-load simulations,
update requirement/capability claims, and verify every operation obeys the
same lifecycle and scheduler invariants.

## Error Model

Child controllers use non-allocating typed enums. Errors preserve protocol
causes while adding operation, position, target, phase, generation, and
deadline context where practical. Busy, invalid request, stale retained state,
capacity exhaustion, timeout, WKC mismatch, malformed response, and protocol
rejection remain distinguishable.

## Verification Strategy

- Focused state-machine tests cover every submission and terminal error class.
- Production simulations run cyclic PDO/DC traffic while recovery spans
  multiple cycles and assert ordering, no duplicate transmission, and bounded
  control-pool use.
- Lifecycle tests prove readiness is revoked before recovery traffic and is not
  restored by terminal recovery success alone.
- Full CI retains `no_std`, release, Clippy, generated artifact, BPF, and host
  integration gates.

## Rollback

All APIs are additive. Each child can be reverted independently until the final
integration child publishes a unified recovery facade. Existing startup,
cyclic, mailbox, register-request, and product-activation behavior remains the
fallback and must continue to pass its current tests.
