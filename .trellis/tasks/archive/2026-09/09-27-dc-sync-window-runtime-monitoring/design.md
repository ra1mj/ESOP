# Design: DC sync-window runtime monitoring

## Architecture

Extend the existing cyclic DC owner instead of adding a competing control
service:

```text
immutable Startup DcTopology
  -> checked DcSyncWindowConfig (BRD expected WKC)
  -> DcCyclicSync optional sync-window slot
  -> one fixed FramePlan with FRMW + optional BRD
  -> one master RX dispatch and one pending generation
  -> atomic reference/window observation commit
  -> combined DC lock predicate
  -> lifecycle + ProcBuf quality gate
```

`DcCyclicSync` remains non-generic and allocation-free. The optional window
state holds one config, one `DcSyncWindowMonitor`, and staged pending data.

## Public Contracts

Add these core types:

```text
DcSyncWindowConfig
DcSyncWindowMonitor
DcSyncWindowSample
```

`DcSyncWindowConfig::for_topology(...)` derives `expected_wkc` from
`DcTopology::len()` with checked conversion. An explicit constructor remains
available for callers whose validated broadcast set is frozen elsewhere.

`DcCyclicSync::with_sync_window(...)` validates that the FRMW and BRD indices
are distinct and installs the optional monitor. Legacy construction has no
window and preserves existing behavior.

The cyclic owner adds:

```text
sync_window_datagram_plan() -> Option<DatagramPlan>
sync_window_monitor() -> Option<&DcSyncWindowMonitor>
is_locked() -> bool
```

## Wire And RX Contract

The existing FRMW remains unchanged. The optional plan is:

```text
command       BRD
index         configured unique index
address       0x0000_092c
payload       four zero bytes
length        4
expected WKC  configured exact topology count
```

`prepare` zeroes the BRD process-image region and creates one pending record
containing the generation, application time, and empty staged reference/window
observations. `complete` dispatches by exact index, validates the corresponding
plan, and stages the decoded response. Only when every enabled response exists
does it update both monitors, increment the complete sync count, publish the
cycle/time fields, and clear pending ownership.

The BRD word is decoded as:

```text
raw = little_endian_u32(payload)
difference_ns = raw & 0x7fff_ffff
```

The raw word remains diagnostic only. The monitor compares the magnitude with
its configured limit.

## Monitor State And Recovery

`DcSyncWindowMonitor` uses the existing public `DcLockState` vocabulary:

- `Unknown`: no accepted or missing observation.
- `Locking`: good samples exist but the acquisition window is incomplete.
- `Locked`: the configured consecutive-good window is complete.
- `Degraded`: a bad/missing sample exists but the loss window is incomplete.
- `Unlocked`: the configured consecutive-bad/missing window is complete.

Entering `Unlocked` increments `loss_count` once. The first subsequent good
sample starts reacquisition; reaching `Locked` after an `Unlocked` episode
increments `recovery_count` once. Missing finalization calls `observe_missing`
and never replaces the last accepted numeric difference.

The lifecycle combined predicate is:

```text
application_reference_monitor == Locked
&& (sync_window_disabled || sync_window_monitor == Locked)
```

Cycle freshness, pending ownership, and last error are still checked by the
quality projector.

## Scheduled Domain Integration

`submit_dc_and_control` builds `FramePlan<2>`, pushes the unchanged FRMW plan,
then pushes the optional BRD plan. Before `prepare`, it validates that both DC
indices are free from Domain ownership, are distinct, and do not alias any
prepared/in-flight control request.

The existing `RxConsumerMux` continues to own dispatch. `DcCyclicSync` accepts
either configured DC index and stages the matching response. The common
`finish_receive` call retires a partial/missing pair exactly once.

## Compatibility And Failure Semantics

- No existing constructor or datagram accessor changes meaning.
- A no-window instance keeps one datagram, immediate FRMW monitor commit, and
  the old lock predicate.
- Configuration and index conflicts fail before `prepare` mutates the process
  image or pending generation.
- Once preparation succeeds, every TX/RX path must call the existing finalizer.
- Partial responses never become current-cycle evidence.
- The monitor reports evidence only; it does not reprogram ESC registers or
  claim recovery of hardware state.

## Verification Shape

Core unit tests own state, decoding, staging, and typed failures. Scheduled
domain tests own frame construction, index partitions, WKC, and shared RX.
Lifecycle/ProcBuf tests own fail-closed projection. Linux simulated-port tests
own the public multi-datagram path. Repository gates own no-std, feature
matrices, generated evidence, BPF builds, and exact pushed-SHA CI.
