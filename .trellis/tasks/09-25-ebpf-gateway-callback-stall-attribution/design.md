# Design: eBPF gateway callback stall attribution

## Scope and boundaries

This increment observes synchronous callback execution owned by the Zenoh
gateway. It covers command subscriptions and query handlers, including the
typed query adapter. It does not measure Zenoh internal queueing, outbound
publish, ROS2, recorder, or real-time EtherCAT execution.

## Gateway marker contract

Export two new stable symbols under the existing `zenoh` feature:

- `esop_zenoh_gateway_callback_begin_v1(request_id: u64, route_kind: u32)`
- `esop_zenoh_gateway_callback_end_v1(request_id: u64, route_kind: u32,
  outcome: u32)`

Callback outcomes are fixed: completed is `0`, abandoned/unwound is `2`. The
gap keeps detail encoding compatible with publish outcomes while route kind
disambiguates callback and publish interpretation.

Rename the private publish request sequence to a gateway operation sequence
and use it for both observation classes. A callback guard invokes begin at
construction, supports one explicit completed finish, and emits abandoned from
`Drop` when unwinding bypasses normal completion.

`invoke_observed_callback(kind, callback, argument)` owns the guard and calls
the supplied `Fn`. `register_subscriber` wraps Command callbacks and
`serve_queries` wraps Query callbacks before passing them to Zenoh. Since
`serve_typed_queries` performs decode, provider work, encode, and reply wait in
the callback supplied to `serve_queries`, the entire typed handler remains
inside one observation.

## BPF operation contract

The existing 1024-entry LRU map remains unchanged in size and key. Reuse the
state's reserved word as a fixed operation class:

- `1`: publish
- `2`: callback

Refactor begin/end logic into verifier-friendly inline helpers accepting the
expected class and route range. Publish accepts routes `0..=2`; callback accepts
routes `3..=4`. State deletion still occurs before end validation or evidence
emission. A callback end cannot complete publish state and vice versa.

The new programs are:

- `esop_gateway_callback_begin`
- `esop_gateway_callback_end`

They emit the existing `UserZenoh/GatewayStall` event with the existing gateway
threshold and aggregate gateway probe statistics. No map, context, statistics,
or event size changes are required.

## Runtime attachment

Reserve attach bits 14 and 15 for callback begin/end. Add exact callback symbol
constants and probe specifications. Refactor the publish attachment into one
private pair helper used by:

- `attach_gateway_publish_probes(target, pid, required)`
- `attach_gateway_callback_probes(target, pid, required)`

The helper validates explicit PID, provides idempotent required-mask promotion,
attaches both programs, detaches the first link on second failure, and only
publishes both capability bits after complete success.

## Evidence and classification

The fixed evidence ABI and classifier are unchanged. Callback evidence uses
route `Command` or `Query`; detail bits `4..5` carry completed or abandoned.
The classifier still requires non-zero threshold, strict duration crossing,
`observed_value == duration_ns`, and a transport-risk cycle. Cross-thread
completion is not expected for synchronous callbacks, but the shared BPF
helper retains honest TID-zero behavior if it occurs.

## Compatibility and rollback

- No-std builds do not compile runtime markers.
- Existing publish symbols and capability bits remain stable.
- Callback public signatures stay unchanged; wrappers move the original
  callback into an equivalent Zenoh callback closure.
- Removing callback attachment leaves the marker calls as deterministic no-op
  functions and preserves the kernel baseline.
- Rollback can remove callback wrappers/programs/bits without changing the
  publish operation map or fixed event ABI.

## Validation strategy

Unit tests cover shared ID allocation, callback completion and unwind cleanup,
route/class validation constants, capability pair masks, and unchanged ABI
sizes. The exact-symbol integration test calls all four gateway marker symbols.
Existing typed/live query and command tests verify callback semantics. BPF
syntax plus a real Clang object build verify the new uprobe programs.
