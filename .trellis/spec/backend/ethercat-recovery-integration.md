# EtherCAT Recovery Integration

## 1. Scope / Trigger

Use this contract when supervision, diagnostics, or a recorder needs one
allocation-free observation surface for explicit `request_state`, `rescan`,
and `reconfigure_slave`. The integration is read-only: each operation-specific
controller remains the protocol authority and the production scheduler remains
the only wire-work owner.

## 2. Signatures

```rust
let status = ExplicitRecoveryStatus::from(operation_status);
let result = ExplicitRecoveryResult::from(operation_result);

let mut diagnostics = ExplicitRecoveryDiagnostics::<64>::new();
diagnostics.observe(cycle, timestamp_ns, status);
let event = diagnostics.pop();
```

The public fixed-size types are `ExplicitRecoveryKind`,
`ExplicitRecoveryPhase`, `ExplicitRecoveryFault`, `ExplicitRecoveryStatus`,
`ExplicitRecoveryResult`, `ExplicitRecoveryDiagnosticCode`,
`ExplicitRecoveryDiagnosticEvent`, and `ExplicitRecoveryDiagnostics<N>`.

## 3. Contracts

### Snapshot Contract

- Unified status variants contain the complete original `StateRequestStatus`,
  `RescanStatus`, or `ReconfigureSlaveStatus`; callers can match the variant to
  recover all operation-specific detail.
- Common status accessors project kind, stable sequence, coarse phase,
  generation, absolute deadline, optional position/station, and exact typed
  fault without allocation or formatting.
- `Idle`, `Complete`, and `Faulted` map directly. Every internal active
  reconfiguration phase maps to coarse `Active`; the exact PDO/watchdog/
  mapping/DC phase remains in the wrapped value.
- Unified result variants preserve the original values. Result generation,
  deadline, completion time, position, and station accessors return `Option`
  where an existing operation-specific result intentionally did not retain a
  field. Do not add guessed metadata or mutate existing public result structs.

### Diagnostic Contract

- `observe` owns one last immutable status per recovery kind and emits only a
  new sequence or changed status.
- A new nonterminal sequence emits `Submitted`; another changed nonterminal
  snapshot emits `Progress`; terminal phases emit `Completed` or `Faulted`.
- An identical snapshot emits nothing. On ring overflow, increment
  `lost_events`, advance the remembered observation, and return immediately so
  the same lost transition is not counted on every cycle.
- Every event retains the complete unified status and therefore the exact child
  fault. Diagnostic code is a projection, not a replacement error taxonomy.
- Observing or draining diagnostics must never alter a controller, retained
  Startup evidence, scheduler ownership, lifecycle gates, or motion permits.

### Scheduler and Lifecycle Contract

- Domain and cyclic DC work execute before the production service slot.
- Fixed recovery priority is Rescan, Reconfigure Slave, then State Request.
- At most one prepared/in-flight control request is owned; `InFlight` remains
  retained across cycles and is never retransmitted merely because a response
  is delayed.
- Status/result conversion and event recording cannot restore topology,
  configuration, drive, DC, command-age, CiA 402, safety, or permit gates.
- Completion does not retry, rescan, reconfigure, return SAFEOP/OP, or refresh
  a command. Application policy must explicitly submit any later operation.

## 4. Validation and Error Matrix

| Condition | Required behavior |
|-----------|-------------------|
| Existing status/result supplied | Wrap the complete value without allocation or protocol mutation |
| Status contains a child error | Return the exact typed child error from `fault()` |
| Result intentionally omits common metadata | Return `None`; never synthesize generation, deadline, completion time, or target data |
| Same status observed again | Return `false` and emit no duplicate event |
| New active sequence observed | Emit one `Submitted` event |
| Same sequence changes while active | Emit one `Progress` event per distinct status |
| Complete or faulted status observed | Emit `Completed` or `Faulted` with the full status |
| Event ring is full | Return `false`, increment `lost_events`, remember the status, and never retry protocol work |
| Invalid ring capacity | `new()` follows `SpscRing`: capacity must be a nonzero power of two |

## 5. Good, Base, and Bad Cases

- Good: observe an active state-request status, drain one `Submitted` event,
  observe changed progress, then drain one `Progress` event with the original
  typed status still available.
- Base: wrap a successful reconfiguration result and read its kind, sequence,
  position, station address, and completion time; absent generation/deadline
  remain `None` because the original result does not retain them.
- Bad: flatten child errors into numeric codes, infer missing result metadata,
  use event completion as lifecycle readiness, or retry a request after event
  ring overflow.

## 6. Tests Required

- Core: every status/result conversion, common accessor, typed-fault retention,
  submitted/progress/completed/faulted event classification, unchanged-status
  deduplication, ring overflow, and full recovery priority order.
- Linux integration: LRW/FRMW before service work, one control slot, delayed
  response without retransmission, true post-receive deadline evidence, unified
  status/result/event use for all three operations, and no lifecycle bypass.
- Repository: core `no_std`, focused tests, `make test-hil`, `make ci`,
  capability validation, and exact pushed GitHub Actions.

## 7. Wrong vs Correct

Wrong:

```rust
if diagnostics.observe(cycle, now, status) == false {
    controller.retry();
}
```

`false` can mean either an unchanged snapshot or a full event ring. It is not
a protocol failure and must never drive recovery state.

Correct:

```rust
let _recorded = diagnostics.observe(cycle, now, status);
let lost = diagnostics.lost_events();
```

Treat the event stream and loss count as observation evidence only. Submit any
later recovery operation through the operation-specific controller and product
policy.

## 8. Qualification Boundary

Deterministic tests prove software ownership, ordering, bounded capacity, and
diagnostic behavior only. They do not prove physical response provenance,
real-slave interoperability, target WCET/jitter, long-duration operation, HIL,
ETG conformance, safe outputs, STO, or functional-safety qualification.
`REC-001` and the capability remain partial until the applicable physical
evidence exists.
