# Design: Generated PDO Plan and Exact Readback

## Architecture Boundary

This task connects two existing layers without adding a second protocol stack:

1. `esop-product-config` converts generated static PDO expectations into one
   fixed-capacity per-slave `PdoConfigPlan`.
2. `esop-ethercat-core::PdoConfigController` executes that plan as alternating
   SDO download and upload-verification steps.
3. The caller continues to own mailbox transport, service scheduling, retries,
   and the overall CONFIGURING lifecycle transition.

The production scheduler is intentionally not changed here. This keeps the
protocol contract independently testable before service arbitration and
mailbox ownership are expanded.

## Controller State Model

Add a public fixed-size step enum:

```text
PdoConfigStep::Download
PdoConfigStep::VerifyUpload
```

`PdoConfigAction` carries this step. The controller stores the current step and
uses the same `SdoTransfer` instance for both directions.

State flow for one plan operation:

```text
Download/Sending
  -> download action
  -> download response complete
VerifyUpload/Sending
  -> upload action
  -> upload response advanced ...
  -> upload response complete
  -> exact length/value comparison
  -> next operation Download/Sending, or Complete
```

`operation_index` identifies the planned write, not an individual mailbox
transaction. It is incremented only after verification succeeds. Existing
configuration and request deadlines cover both halves of the operation.

## Readback Comparison

The planned write remains the source of truth. After a verification upload
completes:

1. Compare `SdoTransfer::data_len()` with `PdoSdoWrite::data_len`.
2. Compare each returned byte with the planned payload.
3. On mismatch, publish either a typed length mismatch or the first byte
   mismatch with byte offset, expected value, and actual value.

The comparison happens before any operation index or phase publication. Failure
uses the existing controller `fail` path, so pending state is cleared and the
fault is latched consistently.

## Product Plan Builder

`StaticProductConfig` gains a builder returning a small owning target object:

```text
ProductPdoStartupPlan<OPS> {
    station_address,
    plan,
}
```

The object exposes station-address and plan accessors plus `into_plan()` for
passing ownership to `PdoConfigController::start`.

The builder performs bounded scans over generated PDO entries:

1. Resolve the configured slave and validate each selected PDO belongs to its
   Domain.
2. Record each first-seen mapping object and its SyncManager in a fixed array.
3. For each first-seen SyncManager, clear `0x1C10 + sm` subindex zero before
   touching any mapping object assigned to that SyncManager.
4. Gather each mapping object's ordered entries into a fixed scratch array and
   call the existing transactional `append_mapping` API.
5. Publish the SyncManager's ordered mapping indexes and final assignment count.
6. Return the target object only after the entire plan succeeds.

The shared scratch limit is the cfggen hard limit of 256 PDO entries per
Domain. A slave belongs to one generated Domain, so this is also a valid upper
bound for one slave. The limit becomes a public product-config constant and
cfggen consumes the same constant, avoiding duplicated capacity values.

## Validation Rules

- SyncManager must be in `0..=15`, the object range representable by
  `0x1C10..=0x1C1F`.
- One mapping object may not change SyncManager or PDO direction within a
  slave.
- A PDO's generated Domain must match its configured slave Domain.
- Per-slave entry/mapping scratch capacity and caller-selected plan capacity
  are checked before publication.
- Mapping/assignment append errors are preserved in a product-level typed
  wrapper.
- A configured slave with no PDO entries returns an empty plan; this supports
  explicit mailbox-only products without inventing a mapping.

## Compatibility

- `PdoConfigAction` gains a field and is constructed only by the controller in
  the current workspace; this is an intentional source-level API extension.
- Existing `PdoConfigPlan` construction remains valid.
- Generated artifact field names and schema versions do not change.
- The cfggen semantic model and SHA-256 inputs do not change.
- Runtime configuration takes more mailbox requests: two transfer sequences per
  planned write. Callers must budget configuration deadlines accordingly, but
  cyclic PDO behavior is unchanged.

## Testing Strategy

Core unit tests build protocol-correct CoE responses and cover exact, length,
value, stale-action, timeout, and segmented-upload paths. Product-config tests
assert exact plan bytes and ordering for repeated mapping indexes across
different slaves. cfggen golden tests ensure the generated module remains
stable and compilable.

## Rollout and Rollback

The feature is additive at the product builder boundary but strengthens
`PdoConfigController` semantics. Callers using that controller will now need to
service verification uploads before completion. Rollback is a single coherent
revert of the controller step model and product builder; no persistent state or
schema migration is involved.

## Qualification Boundary

Passing these tests proves deterministic software plan generation and exact
comparison of supplied SDO responses. It does not prove transport integration,
the authenticity of a response, real-device behavior, timing, or functional
safety. Those require production scheduler/mailbox integration and HIL evidence.
