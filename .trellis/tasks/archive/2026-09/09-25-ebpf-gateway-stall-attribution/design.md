# Design: eBPF gateway stall attribution

## Scope and boundaries

This increment instruments the Zenoh gateway publish path only. The gateway
owns operation lifecycle markers, the BPF program owns bounded duration
measurement, the Rust eBPF runtime owns policy and attachment, and the agent
owns lifecycle classification. The observation path is optional and remains
strictly downstream of the real-time control path.

## Architecture

```text
ZenohGateway::publish
  -> begin_v1(request_id, route_kind)
  -> session.put(...).await + health update
  -> end_v1(request_id, route_kind, outcome)
       |
       v
optional uprobe pair
  -> LRU in-flight map keyed by { tgid, request_id }
  -> elapsed time and policy checks
  -> fixed RuntimeEvidence ring-buffer event
       |
       v
Rust decoder -> correlated lifecycle classifier -> GatewayStall incident
```

## Gateway marker contract

The Zenoh-enabled runtime exports these stable symbols:

- `esop_zenoh_gateway_publish_begin_v1(request_id: u64, route_kind: u32)`
- `esop_zenoh_gateway_publish_end_v1(request_id: u64, route_kind: u32,
  outcome: u32)`

Both functions are `extern "C"`, unmangled, non-inlined marker bodies. Arguments
are consumed through `black_box` so link-time optimization cannot erase the
observable call boundary. A process-global atomic sequence allocates non-zero
request IDs.

A small publish guard calls `begin_v1` at construction and owns the matching
`end_v1`. Normal completion records success or transport failure explicitly;
`Drop` records cancellation when the async future is abandoned. The guard
prevents double completion. This contract measures the caller-visible publish
operation, including post-put health accounting, rather than future creation.

Route kind occupies the low detail bits. Outcome occupies fixed high detail
bits and distinguishes success, transport failure, and cancellation. Unknown
values are rejected by BPF instead of being coerced.

## BPF contracts

### Configuration

Append the following fields to the existing runtime context:

- `gateway_stall_threshold_ns: u64`
- `gateway_probe_epoch: u32`
- reserved alignment field

The expected C/Rust context size becomes 160 bytes. An update increments the
epoch and writes the whole context atomically. A zero threshold is invalid.

### In-flight map

`ESOP_GATEWAY_OPERATIONS` is an LRU hash with 1024 entries.

Key:

```text
request_id: u64
tgid:       u32
reserved:   u32
```

Value records start time, policy epoch, start TID, and route kind. Including
TGID avoids request-ID collision across gateway processes. TID is deliberately
not part of the key because an async task may resume on another worker thread.

### Begin probe

The begin probe reads fixed register arguments, validates non-zero request ID,
route range, configured target PID, and context availability, then inserts the
bounded state. Failed or duplicate inserts increment a mismatch/loss counter.

### End probe

The end probe finds state by current TGID and request ID, validates route and
epoch, and deletes state on every resolved path. It computes elapsed time and
emits only when `duration_ns > gateway_stall_threshold_ns`.

The emitted event uses:

- domain `UserZenoh`
- kind `GatewayStall`
- severity `Error`
- `evidence_id = request_id`
- `observed_value = duration_ns`
- `threshold = gateway_stall_threshold_ns`
- `duration_ns = duration_ns`
- `pid = tgid`
- `tid = start_tid` only when start and end TID match, otherwise `0`
- CPU and cycle context from completion time
- fixed route/outcome detail byte

The event remains 96 bytes. A migrated async task is therefore process-
attributed without falsely claiming one worker thread.

### Statistics

Append four counters to the BPF and Rust statistics layouts:

- gateway probe begins
- gateway probe completions
- gateway stalls
- gateway probe mismatches

The expected statistics size becomes 208 bytes. Host aggregation uses
saturating addition.

## Runtime attachment

`BpfRuntime::attach_gateway_publish_probes(target, pid, required)` loads and
attaches the named begin/end BPF programs to the exact marker symbols. The pair
is transactional: if the second attach fails, the first link is detached.

Two capability bits represent the pair. They are not part of the default
kernel tracepoint mask because the target path is supplied after object load.
On optional failure, the method returns `Ok(false)` and leaves the bits absent.
On required failure, it returns a typed runtime error. On success, both bits are
published in the capability snapshot; required mode also extends the required
mask.

This makes `USER_PROBE_UNAVAILABLE` observable without degrading the default
scheduler/process tracepoint baseline.

## Classification

`GatewayStall` classification requires all of the following:

1. evidence and cycle are correlated by the existing cycle rules;
2. the cycle has transport risk;
3. threshold is non-zero;
4. `duration_ns > threshold`;
5. `observed_value == duration_ns`.

Valid evidence produces the existing controlled-stop gateway incident at error
severity and confidence 75. Any failed predicate returns no incident.

## Compatibility

- The marker module is compiled only with the existing `zenoh` feature; the
  gateway's default no-std surface is unchanged.
- New fields and counters are append-only with compile-time size checks.
- Existing evidence enum values and 96-byte wire layout remain unchanged.
- Architecturally correct probe argument extraction is implemented for the
  repository's qualified x86_64 BPF build. Other target architectures remain
  unclaimed until compiled and attached there.

## Failure handling and rollback

- Missing marker symbols or BPF programs: optional attach reports unavailable;
  required attach fails before capability readiness is claimed.
- Partial pair attach: detach the first link and expose neither capability bit.
- Stale state after policy change: epoch mismatch deletes it without evidence.
- Lost end event/process death: LRU bounds memory; process-exit tracking remains
  separate and no control action is taken from stale entries.
- Rollback is removal of the optional attachment call or disabling its mask;
  kernel tracepoint observation continues unchanged.

## Validation strategy

Unit tests cover marker symbol linkage, request/outcome helpers, configuration,
ABI sizes, decode semantics, attachment masks, saturating statistics, and
classification predicates. BPF syntax and object builds verify both fallback
headers and the clang BPF path. Existing workspace/no-std tests guard
compatibility, and live Zenoh tests exercise the async publish lifecycle when a
local router is available.
