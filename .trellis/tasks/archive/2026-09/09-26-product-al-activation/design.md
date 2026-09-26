# Design: PREOP-Gated Product Activation

## Architecture Boundary

Keep the existing state machines as owners of their protocols:

```text
StartupController
  scan -> SII identity -> AL to PREOP
  -> AwaitingConfiguration

ScheduledProductionServiceScheduler
  PDO Configuration -> Mapping -> DC Configuration
  -> release Startup barrier from actual Complete phases

StartupController
  retained SlaveTable -> AL to SAFEOP -> AL to OP -> Ready
```

Startup continues to own scan, identity, slave order, AL actions, status polls,
and final readiness. The scheduler continues to own service priority and the
single control request. Configuration controllers remain unchanged.

## Configuration Contract

Add a small copyable service requirement value in `startup.rs` with boolean
membership for PDO, Mapping, and DC. `StartupConfig::new` uses an empty value;
a builder opts into the PREOP barrier.

The startup FSM stores two targets:

- final target: the existing caller target, SAFEOP or OP;
- current stage target: PREOP before release, then the final target.

`start_al_for_current` always asks `AlTransitionController` for the current
stage target. `finish_al_step` advances legal intermediate states exactly as
today. After the last slave reaches PREOP, startup enters
`AwaitingConfiguration` instead of Ready.

## Scheduler Release

Before ordinary selection, the scheduler checks whether Startup is awaiting
configuration. It reads the frozen requirement set and validates each required
binding:

- absent binding -> typed `MissingController(service)` error;
- controller Complete -> requirement satisfied;
- any other phase -> barrier remains closed.

When all required controllers are Complete, the scheduler calls a crate-visible
startup release method with the current monotonic time. Startup resets only its
slave cursor and AL subcontroller, retains scan/SII/table evidence, and becomes
active again. Fixed priority then selects Startup for post-configuration AL.

The check occurs at the beginning of the next production cycle. Callers that
must run multiple per-slave jobs must restart the relevant controller before
that next call; automatic batch iteration is a later product-config task.

## Progress and Errors

Add `StartupPhase::AwaitingConfiguration` and
`StartupProgress::AwaitingConfiguration`. No control request is allocated in
that phase. Invalid barrier/target combinations return a typed Startup error
before mutation. A release outside the waiting phase fails closed.

Production cycle reports continue using Startup, PDO, Mapping, and DC service
kinds. A barrier wait is represented by the Startup progress when PREOP is
reached; subsequent cycles select the required configuration service. Every
report also carries the actual optional Startup phase so lifecycle projection
keeps Topology false during those configuration-service cycles and only marks
it ready after Startup reaches Ready.

## Compatibility

- Empty requirements preserve the old `StartupConfig::new` path.
- Enum additions are additive inside this workspace; exhaustive matches are
  updated explicitly.
- No constructor arity changes are required.
- Existing scheduler service priority is unchanged; only an awaiting Startup
  is intentionally considered inactive until release.

## Testing Strategy

Core startup tests cover one/multiple slaves, PREOP barrier entry, retained
records, legal SAFEOP/OP progression, invalid configuration, and restart.
Production scheduler tests cover missing bindings, non-complete services,
automatic all-complete release, service priority, request ownership, and AL
fault blocking. Linux simulated integration proves the end-to-end ordered flow
without rescanning and lifecycle readiness behavior.

## Rollout and Rollback

The feature is opt-in and changes no generated schema or persistent state.
Rollback removes the requirement value and barrier phase while leaving all
configuration services and the prior PDO scheduler integration intact.

## Qualification Boundary

Observed AL SAFEOP/OP status in simulation is software evidence, not proof of
valid physical PDO exchange, authentic slave behavior, measured timing, or HIL.
