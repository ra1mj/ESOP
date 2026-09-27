# EtherCAT Explicit Recovery Control Plane

## Goal

Implement the software-side portion of `REC-001`: explicit, bounded EtherCAT
recovery operations for `request_state`, `rescan`, and `reconfigure_slave`.
Recovery must remain a control-plane activity that preserves the deterministic
PDO cycle, never returns motion to OP implicitly, and publishes typed evidence
instead of hiding failures behind automatic retries.

## Source Requirements

- `docs/ethercat-master-requirements.md` `REC-001` requires explicit
  `rescan`, `reconfigure_slave`, and `request_state` operations plus evidence
  that recovery work does not break the cyclic budget.
- `docs/esop-etg-cia402-master-requirements.md` requires bounded asynchronous
  recovery APIs and keeps automatic recovery application-configured and
  disabled by default.
- Existing startup, AL transition, SII, PDO/SM/FMMU/DC, lifecycle, scheduler,
  and diagnostics state machines remain the protocol authorities. Recovery
  APIs must compose them rather than duplicate protocol behavior.

## Functional Requirements

1. Provide explicit asynchronous `request_state(position, target, deadline)`,
   `rescan(deadline)`, and `reconfigure_slave(position, deadline)` control
   operations with fixed-capacity state and stable typed outcomes.
2. Execute at most the configured bounded amount of recovery work per cycle;
   never allocate, block, sleep, parse unbounded data, or spin in the cyclic
   path.
3. Keep cyclic Domain/DC send and receive processing authoritative while a
   recovery operation is pending or in flight. Recovery datagrams use the
   existing control pool and RX finalization contracts.
4. `request_state` shall use the existing AL transition controller, verified
   station address, current AL evidence, generation, WKC, timeout, and Device
   Emulation acknowledgement policy.
5. `rescan` shall be explicit, invalidate stale topology/configuration evidence
   before scanning, and reuse the startup scan/SII/topology pipeline.
6. `reconfigure_slave` shall target exactly one verified slave and reuse the
   existing SM/FMMU/PDO/DC configuration authorities without silently
   reconfiguring unrelated slaves.
7. Operations that can invalidate motion readiness shall revoke the relevant
   lifecycle evidence before their first wire action. Completion must not
   implicitly publish OP readiness or fresh motion commands.
8. Reject duplicate active operations, invalid positions/targets/deadlines,
   stale generations, unavailable control capacity, and inconsistent retained
   evidence with deterministic typed errors.
9. Expose immutable progress/result snapshots suitable for diagnostics without
   formatting, allocation, or mutable access to protocol internals.
10. Keep automatic recovery policy out of the EtherCAT core. Applications may
    explicitly submit another operation after observing a terminal result.

## Child Deliverables

1. **EtherCAT runtime state requests**: explicit bounded `request_state` path,
   production scheduler integration, retained-state reconciliation, lifecycle
   gating, and Linux simulation tests.
2. **EtherCAT explicit rescan**: named control operation over the startup
   scan/SII/topology path, including stale-evidence invalidation and cyclic
   coexistence tests.
3. **EtherCAT single-slave reconfiguration**: targeted configuration plan and
   bounded execution over existing SM/FMMU/PDO/DC controllers.
4. **Recovery integration and qualification**: unified API/status surface,
   diagnostics, documentation/capability updates, and mixed cyclic-load budget
   evidence for all three operations.

## Safety and Determinism Requirements

- No runtime heap allocation, recursion, unbounded retry, or implicit recovery.
- A timeout, WKC mismatch, AL error, generation mismatch, link/topology change,
  or capacity failure terminates the requested operation fail-closed.
- Recovery completion is communication evidence only. It does not prove safe
  outputs, STO, physical response origin, hardware timing, HIL, ETG
  conformance, or functional-safety qualification.
- Entering or remaining in OP requires the existing application/lifecycle,
  health, DC, command-age, and CiA 402 gates; a recovery API cannot bypass
  them.
- Existing steady-state PDO behavior and public APIs remain compatible unless
  a child task documents an additive API.

## Acceptance Criteria

- [x] All four child deliverables are completed and archived with focused unit
      and Linux simulation coverage.
- [x] `request_state`, `rescan`, and `reconfigure_slave` are explicit bounded
      control-plane operations with typed progress and terminal results.
- [x] Tests prove recovery work is scheduled after the cyclic Domain/DC path,
      consumes bounded control capacity, and does not restart or replace PDO
      processing implicitly.
- [x] Tests prove lifecycle/configuration evidence is revoked before recovery
      can invalidate motion readiness and is restored only by existing verified
      gates.
- [x] Automatic recovery and automatic return to OP remain disabled by default.
- [x] Requirements, capability manifest, software PRD, and Trellis specs state
      the implemented software boundary and remaining hardware qualification
      gaps consistently.
- [x] `make ci`, target/no_std checks, focused simulation tests, and exact
      GitHub Actions for the delivered commit succeed.

## Closeout Evidence

- Archived child tasks cover runtime state requests, explicit rescans,
  single-slave reconfiguration, and recovery integration/qualification.
- Delivered software commit `bcb9c9e14bfeabbe32dfc07d0bf0a5a2ba0c7112`
  passed exact GitHub Actions quality run `36340589505`, including Rust,
  Zenoh, BPF build, and privileged eBPF runtime qualification jobs.
- The capability remains `partial`: deterministic software evidence does not
  claim target WCET/jitter, physical response provenance, real-slave/HIL
  interoperability, ETG conformance, safe outputs, STO, or functional safety.

## Out of Scope

- Application recovery policy, retry orchestration, or automatic OP return.
- FSoE, STO control, safe-output certification, or functional-safety claims.
- Real-slave interoperability, target WCET/jitter qualification, HIL, or ETG
  conformance evidence; these require the planned hardware environment.
- Hot-plug process-image resizing or dynamic product reconfiguration outside
  the compiled fixed-capacity product contract.
