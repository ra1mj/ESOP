# EtherCAT Runtime Slave Monitoring

## 1. Scope / Trigger

Use this contract for periodic, read-only AL/DL supervision of a frozen verified
station list. The monitor is an observer; it cannot change AL state, repair a
slave, rescan topology, or authorize motion. It executes through the production
service slot after mandatory configuration/DC/state requests. Optional monitor,
mailbox, runtime SDO and diagnostic register work share bounded round-robin
selection.

## 2. Signatures

```rust
let mut monitor = SlaveMonitorController::<32, 64>::new(
    42, &stations, cadence_ns, max_age_ns, request_timeout_ns, now_ns,
)?;
let services = ScheduledProductionServices::<32, 0, 0>::new(None, None, None, None)
    .with_slave_monitor(&mut monitor);
let snapshot = monitor.snapshots();
let quality = monitor.quality(now_ns); // SlaveMonitoringService trait
let event = monitor.pop_event();
```

`SlaveMonitoringService` is object-safe and exposes `is_due`, `pending`,
`next_action`, `accept_completed`, `quality`, and `observe_startup_fault`.
`SlaveMonitorAction` binds token/index/generation/sample sequence, position,
station, AL/DL kind, exact physical address, length and absolute deadline.

## 3. Contracts

- Freeze each position, nonzero station address, required AL state and raw DL
  Status mask before activation. The caller obtains stations from verified
  Startup evidence. Duplicate positions or station addresses are rejected.
- Read AL Status plus Status Code as one exact six-byte FPRD, then DL Status as
  one exact two-byte FPRD for the same sample. Use ordinary exact WKC 1 and exact
  fixed addresses, including at `ControlRequest` completion and immutable-action
  comparison. Neither read uses capability-probe exceptions.
- Publish a station snapshot only after the complete pair. Age begins at the
  earlier AL observation; future timestamps and age exceeding `max_age_ns` are
  unqualified. Unknown, stale, wrong required AL state, AL Error Indication,
  nonzero AL code, or missing required DL bits cannot qualify topology.
- A newly observed bad AL blocks quality immediately while the old published
  pair remains unchanged. A terminal read error invalidates that station,
  records typed contextual evidence, clears staged data and advances to the
  next station. It does not latch the scheduler to this observer.
- Cadence is the minimum delay after each terminal read, including failures.
  Terminal actions release ownership to the shared optional-service cursor,
  so a saturated monitor cannot starve queued mailbox/SDO/register work.
  An already in-flight control request retains sole ownership across cycles.
- Every production cycle report contains optional `SlaveMonitorQuality`,
  regardless of the selected service. The lifecycle adapter always ANDs it
  into topology; Idle or lower-priority work cannot bypass stale observations.
  Platform, drive, supervisor, command, deadline and external safety remain
  with their original owners. Monitoring never grants or renews permits.
- Events retain position/station, timestamp and the read action (generation,
  deadline and kind). AL and DL fault variants retain their raw typed status.
  Startup AL faults preserve the complete original `StartupAlFault`, are
  deduplicated by immutable record, and carry no invented wire generation.
  A full or zero-capacity event ring increments `lost_events` and returns.

## 4. Validation & Error Matrix

| Condition | Result |
| --- | --- |
| Empty/oversized station list | `InvalidCapacity` |
| Zero cadence/age/timeout or initial deadline overflow | `InvalidTiming` |
| Station zero / unknown required AL state | `InvalidStation` / `InvalidRequiredState` |
| Duplicate position or station | `DuplicateStation` |
| Substituted owner/action/policy/deadline | `StationFailed(_, ActionMismatch)` |
| Timeout, WKC, length or TX error | `StationFailed(_, Control(original_error))` |
| AL or DL bad, full sample | Snapshot retained, quality false, typed event |
| Event overflow | Lost count increments; transport and publication continue |

## 5. Good / Base / Bad Cases

- Good: AL OP/code 0 followed by matching DL bits publishes one known sample.
- Base: AL is staged, but no first snapshot exists until DL completes.
- Bad: A slave disappears during DL; the old station sample is invalidated,
  the next station remains serviceable, and lifecycle topology is blocked.
- Bad: A later cycle selects a mailbox while monitor data ages out; the
  report's monitor quality still blocks topology without changing CoE policy.

## 6. Tests Required

`crates/esop-ethercat-core/tests/slave_monitor.rs` verifies pair publication,
earlier timestamp age, partial-read rejection, immediate bad AL, station-local
failure advance, immutable action checks and bounded event overflow.
`production_service` tests verify in-flight ownership, optional-service
round-robin fairness, release on TX failure and request-local failure. Lifecycle
tests verify quality can only clear topology and preserves every other caller fact.

## 7. Wrong vs Correct

Wrong: treat the latest AL read alone as a fresh complete station snapshot,
or gate monitoring only when `selected() == SlaveMonitor`.

Correct: publish paired AL/DL evidence, timestamp it from the earlier read,
and apply `cycle.slave_monitor_quality()` on every production cycle report.
