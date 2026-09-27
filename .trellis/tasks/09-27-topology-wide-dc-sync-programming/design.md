# Design: topology-wide DC SYNC programming

## Architecture

Add three contracts without changing the legacy single-station controller:

```text
selected ESI/SII mode + product base period
  -> shared DcSyncTiming resolver
  -> generated, hashed per-slave resolved timing
  -> StaticProductConfig::dc_sync_plan()
  -> DcSyncController + immutable Startup DcTopology
  -> bounded production service + PREOP barrier
  -> complete position-keyed programmed evidence
```

`DcSyncTiming`, `DcSyncPlanEntry`, `DcSyncPlan<MAX_SLAVES>`, and
`DcSyncController<MAX_SLAVES>` live in `esop-ethercat-core` so host cfggen,
`no_std` product configuration, production scheduling, and public tests share
one wire-register contract.

## Timing Resolution

The resolver consumes `base_period_ns: u64` and `SiiDcMode`.

For SYNC0:

```text
cycle0 = direct CycleTimeSync0, when nonzero
       = base_period * factor, when factor > 0
       = base_period / abs(factor), when factor < 0 and exactly divisible
       = error, otherwise
```

`AssignActivate` is retained exactly. Its high byte is the DC activation
register at `0x0981`; required product modes must contain the DC-unit and SYNC0
enable bits. The low byte is the CUC byte at `0x0980`.

If SYNC1 is not active, its factor and shift must both be zero and the resolved
cycle1 register is zero. If active, first calculate the second-event period:

```text
T1 = cycle0 * factor                    when factor > 0
T1 = base_period * abs(factor)          when factor < 0
T1 = max(base_period, cycle0)           when factor == 0
cycle1_register = T1 - cycle0 + shift1
```

Every operation uses checked integer arithmetic and the final cycle registers
must fit `u32`. The repeat period used for common-phase alignment is
`cycle0 + cycle1_register`.

## Generated Product Contract

`ProductSlaveConfig` gains `dc_sync_timing: Option<DcSyncTiming>`. Cfggen
resolves it from the selected `EsiDcMode` and product base period before any
artifact publication. Normalized product JSON and inventory expose a
`dc_sync_timing` object; generated C and Rust expose the same absolute values.

`StaticProductConfig::dc_sync_plan()` rebuilds the resolver result and requires
byte-for-byte equality with the generated timing. It then appends each
DC-required slave to a fixed `DcSyncPlan<SLAVES>` in product order and freezes
the unique configured reference position. A non-DC slave must have neither a
selected mode nor resolved timing.

`startup_profiles()` calls this validation before constructing profiles, so a
tampered hand-authored static configuration cannot reach Startup.

## Controller State Machine

`DcSyncController<MAX_SLAVES>` copies and validates the candidate plan before
publishing any action. Its phases are:

1. `DisablingSync`: write zero to `0x0981` for every entry.
2. `WritingCycles`: write one eight-byte little-endian cycle0/cycle1 payload
   starting at `0x09a0` for every entry.
3. `ReadingReferenceTime`: read eight bytes from reference `0x0910` once.
4. `WritingStartTimes`: write each `common_epoch + shift0` to `0x0990`.
5. `AssigningActivation`: write the exact two-byte `AssignActivate` word to
   `0x0980` for every entry.
6. `Complete` or `Faulted`.

All actions use fixed addressing, exact WKC 1, one current generation, bounded
token/index cursors, request deadlines, and one operation deadline. A zero-entry
plan completes immediately.

## Common Epoch

At `start()`, the controller validates each plan entry against the immutable
topology and computes the checked LCM of every repeat period. When the one
reference-clock read completes:

```text
remaining_budget = 2 * plan_len * request_timeout
lead = max(config.sync_delay_ns, remaining_budget)
base = reference_system_time + lead
common_epoch = next_strict_multiple(base, common_repeat_period)
start_i = common_epoch + shift0_i
```

The strict next multiple provides at least one complete repeat interval after
the lead boundary. Signed shifts use `i128` for range checks. The evidence
records the sampled reference time, common epoch, effective lead, repeat
period, per-slave start, cycles, and exact activation word.

## Publication And Failure Semantics

Plan validation is transactional. The controller stages per-slave evidence as
actions succeed but exposes an empty public slice until every final activation
write is accepted. A terminal error clears public length, latches the first
typed error, and preserves only bounded `completed_count`/phase diagnostics.

Already accepted ESC writes are irreversible. Restart creates a new generation
and clears all staged/public evidence; it does not claim hardware rollback.

## Production Integration

Add `DcSyncConfiguration` to `ScheduledProductionServiceKind`, progress, fault,
selection, request matching, enqueue/accept/timeout, recovery, and readiness.
`ScheduledProductionServices` gains an optional controller and a builder.

`StartupConfigurationServices` gains a dedicated bit. Fixed barrier order is:

```text
PDO -> Mapping -> DC Clock Offset/Delay -> DC SYNC -> legacy DC -> Startup
```

The lifecycle guard treats the new service exactly as the other bounded
configuration services. Existing constructors and legacy controller fields
remain unchanged.

## Compatibility And Rollback

- Existing `DcController`, `DcConfig`, and scheduler constructor signatures are
  unchanged.
- The new product field is additive to the Rust/C/JSON generated contract;
  checked-in golden output changes together with the configuration hash.
- Products with no DC-required slaves build an empty plan and need not require
  the new Startup service.
- If implementation uncovers unsupported factor semantics, roll back to the
  planning phase rather than accepting an approximate conversion.

## Verification Shape

Unit tests own formulas, overflow boundaries, state transitions, and evidence.
Cross-crate tests own generated plan propagation. Production-service and
lifecycle tests own configuration gating. A simulated-port integration test
owns the generic request/RX path and complete two-drive sequence. Repository
quality gates own formatting, no-std, feature matrices, Clippy, release builds,
BPF compilation, and exact pushed-SHA CI.
