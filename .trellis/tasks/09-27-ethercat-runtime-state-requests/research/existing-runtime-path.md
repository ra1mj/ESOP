# Existing Runtime State-Transition Path

## Relevant Authorities

- `crates/esop-ethercat-core/src/al.rs`: one legal ESM step at a time with
  token/generation/deadline/WKC/status-code validation and optional AL-error
  acknowledgement.
- `crates/esop-ethercat-core/src/startup.rs`: retained slave/profile evidence,
  multi-step target traversal, Device Emulation policy, and private OpOnly
  SyncManager sequencing.
- `crates/esop-ethercat-core/src/slave.rs`: fixed-capacity retained records,
  legal next-state graph, requested-state fields, and observation updates.
- `crates/esop-ethercat-core/src/production_service.rs`: one fixed-priority
  service request retained across cycles after Domain/DC processing.
- `crates/esop-lifecycle-guard/src/ethercat.rs`: scheduler-selected service to
  lifecycle-gate projection.
- `crates/esop-ethercat-linux-port/tests/scheduled_domains.rs`: existing
  production-shaped ordering and delayed-response simulation patterns.

## Findings

1. `AlTransitionController` deliberately performs one ESM step. A runtime API
   must orchestrate repeated verified steps instead of changing this authority.
2. `StartupController` already retains verified station, status, ESI transition
   timeouts, request timeout, Device Emulation, and OpOnly profile evidence, but
   its `start` method intentionally destroys that evidence and rescans.
3. Reusing Startup's whole FSM for one runtime state request would conflate
   topology readiness with a single-slave operation and would walk unrelated
   slaves after completion. A dedicated bounded controller is the smaller
   ownership boundary.
4. The production scheduler retains one request handle across cycles, rejects
   action mismatches, releases prepared requests after TX failure, and always
   runs Domain/DC transport before the selected service. The new service should
   extend these exact branches rather than introduce another scheduler.
5. Faulted configuration controllers remain selected because only Idle and
   Complete are inactive. State requests should use the same persistent
   fail-closed behavior because a timed-out AL write leaves the physical state
   uncertain.
6. Lifecycle currently gates Startup on topology and configuration/mailbox
   services on CoE readiness. A runtime ESM change is closest to topology
   readiness and should conservatively clear that gate while active/faulted.
7. Startup's OpOnly sequence is private and intertwined with its transition
   stage. This child must reject such profiles rather than duplicate or bypass
   the safety behavior.

## Implementation Boundary

Create a standalone state-request FSM, a narrow Startup context/reconciliation
adapter, one optional production service binding, and one lifecycle projection.
Do not rescan, mutate PDO/DC configuration, add automatic policy, or claim
physical timing/interop evidence.
