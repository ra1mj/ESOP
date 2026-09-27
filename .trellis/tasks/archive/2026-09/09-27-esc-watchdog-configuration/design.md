# Design

## Boundaries

The new `watchdog` core module owns raw ESC watchdog values, fixed-capacity
plans, controller actions, exact readback, and complete-only programmed
evidence. `esop-cfggen` owns strict host schema validation and deterministic
artifact emission. `esop-product-config` owns static runtime validation and
product-order plan construction. `production_service` owns transport priority
and request routing. `startup` owns only the explicit PREOP completion gate.

No layer calculates an engineering-time duration from the raw register values,
and observed SyncManager/FMMU pages never become desired watchdog intent.

## Product Contracts

Host input uses:

```json
"watchdog": {
  "divider": 2500,
  "process_data_intervals": 100
}
```

Both keys are optional individually, but the object must contain at least one
nonzero value. The generated/runtime representation is:

```rust
pub struct EscWatchdogConfig {
    pub divider: Option<u16>,
    pub process_data_intervals: Option<u16>,
}

pub struct ProductSlaveConfig {
    // existing fields ...
    pub watchdog: Option<EscWatchdogConfig>,
}
```

Absence means no register action. `Some(0)` is invalid at cfggen and runtime
boundaries so default preservation is never encoded as a write.

## Plan and Controller

`WatchdogPlan<MAX_SLAVES>` stores ordered `WatchdogPlanEntry` values containing
position, station address, and exact config. `push` validates the config,
capacity, unique position, and unique station address. Product construction
walks generated slaves in canonical position order and pushes only configured
entries.

The controller copies the plan at `start` and runs this deterministic sequence
for each entry:

```text
divider present?
  -> FPWR station:0x0400 little-endian u16
  -> FPRD station:0x0400 exact 2-byte comparison
process_data_intervals present?
  -> FPWR station:0x0420 little-endian u16
  -> FPRD station:0x0420 exact 2-byte comparison
advance to next product-order entry
```

The phase machine skips absent fields without emitting actions. Every action
carries token, datagram index, generation, position, station address, field,
operation, fixed address, payload/read length, bounded deadline, and expected
WKC 1. Writes expect zero response bytes; reads expect exactly two. The control
pool match contract covers operation, payload, effective datagram length, and
deadline before a response is accepted.

Programmed evidence is staged per entry and becomes visible only after the
whole plan completes. A failed batch retains no published slice. Restart from
Complete or Faulted replaces all plan/staged state and clears the first error.

## Scheduler and Lifecycle

`ScheduledProductionServices` adds an optional
`WatchdogController<MAX_SLAVES>` through
`with_watchdog_configuration`. Scheduler enums gain one Watchdog variant and
all request-match, enqueue, consume, fault, recovery, readiness, and active
selection paths route it exactly like other control-pool services.

Fixed startup-barrier priority becomes:

```text
PDO Configuration
-> Watchdog Configuration
-> Mapping
-> DC Clock Configuration
-> DC SYNC Configuration
-> legacy DC Configuration
-> Startup continuation
```

Outside the barrier, Startup retains its existing top-level priority, while
Watchdog remains ahead of Mapping among independently active services. The new
`StartupConfigurationServices` bit is opt-in, so existing masks are bytewise
compatible and do not acquire new traffic.

## Artifact Compatibility

Adding `watchdog` changes the semantic configuration hash only when the
manifest declares it. The checked-in simulator intentionally updates its two
drive declarations and all six generated artifacts. Manifests without the
field deserialize to `None`, preserve runtime behavior and semantic hash, but
newly generated C/Rust artifacts still use the expanded schema with explicit
absence fields.

The generated C slave structure adds `has_watchdog`, per-field presence flags,
and raw `uint16_t` values; generated Rust imports `EscWatchdogConfig` and emits
the exact `Option` value. Normalized product and inventory serialize the same
object shape.

## Failure and Rollback

Input validation fails before staging output. Runtime plan validation fails
before controller mutation. Controller failures latch the first typed error,
publish no complete evidence, and keep the PREOP barrier closed. Hardware
writes already accepted before a later fault are not described as rolled back;
an explicit controller restart is required. Reverting the implementation
commit restores the prior schema-compatible absence-only behavior but invalidates
artifacts generated with watchdog fields.
