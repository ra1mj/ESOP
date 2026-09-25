# Design: eBPF Linux raw port stall attribution

## Boundary

The observation window begins immediately before the Linux AF_PACKET syscall
and ends immediately after its return value and `errno` are captured. It does
not include frame validation, link-state policy handling, caller scheduling,
driver queue residence, NIC processing, wire time, slave response, RX drain,
or full master-cycle work.

The `esop-ethercat-core` trait and deterministic simulator stay untouched.
Only the Linux development/HIL implementation exports the stable markers.

## Marker Contract

```text
esop_linux_raw_port_operation_begin_v1(ifindex: u32, operation: u32)
esop_linux_raw_port_operation_end_v1(ifindex: u32, operation: u32, outcome: u32)
```

Operation codes are `TX = 0` and `RX = 1`. TX outcomes are success, syscall
error, and partial write. RX outcomes are frame, empty, link-down, and other
error. The low detail nibble stores operation and the high detail nibble stores
outcome. Marker functions are `extern "C"`, unmangled, non-inlined, and contain
only a compiler fence/no-op body so their symbols and argument registers remain
observable without adding application-side timing work.

TX validation happens before begin. Each started call owns a small guard that
closes exactly once with the selected outcome; a defensive drop path closes as
error during unwind. `errno` is captured before the end marker is called.

## BPF State And Flow

`ESOP_RAW_PORT_OPERATIONS` is a 1024-entry LRU hash keyed by the full
`pid_tgid`. Each value stores begin monotonic time, policy epoch, interface
index, and operation. A begin validates the configured threshold, process
filter, positive interface, and operation. It replaces stale same-thread state
with `BPF_ANY` and increments mismatch accounting when replacement was needed.

An end looks up the current thread key, copies state, deletes it, and then
validates:

1. context and nonzero threshold are present;
2. tracked process and policy epoch still match;
3. interface and operation match begin state;
4. outcome is allowed for the operation;
5. monotonic time did not move backwards;
6. elapsed duration is strictly greater than the configured threshold.

Only the final condition emits evidence. The evidence uses:

- domain `UserEsop`;
- kind `RawPortStall`;
- severity `Error`;
- begin timestamp as evidence ID;
- current TGID/TID and end CPU;
- begin interface index in `netdev_ifindex`;
- elapsed time in `observed_value` and `duration_ns`;
- configured threshold and count one;
- operation/outcome in `detail`.

The fixed evidence ABI remains 96 bytes. Context grows from 160 to 176 bytes
with `raw_port_stall_threshold_ns`, `raw_port_probe_epoch`, and a reserved word.
Statistics grow from 208 to 240 bytes with four raw-port counters. C static
assertions and Rust size tests make the bundle version change explicit.

## Runtime API

Attach bits 16 and 17 are reserved for the raw-port begin/end programs. They
remain outside `ATTACH_ALL`, which continues to mean automatically loadable
kernel tracepoints. `attach_raw_port_probes` reuses a generalized transactional
user-probe-pair helper also used by the Zenoh pairs.

`RuntimeConfig::raw_port_stall_threshold_ns` defaults to 1 ms and must be
nonzero. `KernelContext::set_raw_port_tracking` increments the raw-port epoch
before publishing the new threshold. `set_tracked_pid` advances both gateway
and raw-port epochs when the filter changes. Every context update is staged in
a local copy and written to the array map before the in-memory copy changes.

## Correlation

The decoder maps kind 11 to `RawPortStall`. The agent maps a consistent strict
over-threshold sample plus a transport-risk cycle to `HostPortStall`, Error,
`ControlledStop`, confidence 80. This confidence is higher than gateway-stall
correlation because the measurement directly brackets the Linux raw socket
boundary, but it remains below a hard fact because correlation does not prove
causation inside the kernel or network.

No incident is emitted from the kind discriminant alone. The event cannot
grant or revoke a motion permit directly.

## Compatibility And Rollback

- Runtime evidence and protobuf wire fields do not change size or numbering.
- Evidence and incident enum values are appended, preserving existing values.
- Context/stats map values intentionally change size; loader and object must be
  deployed as one bundle and are guarded by compile-time/runtime tests.
- If user symbols are absent, optional attachment returns unavailable without
  changing the kernel baseline. Required attachment fails closed.
- Rollback is the prior loader/object/raw-port binary set; mixed old/new map
  ABIs are unsupported and must not be presented as compatible.

## Operational Limits

This increment requires later target evidence for real symbol attachment,
verifier acceptance, induced blocking/error outcomes, ring-buffer behavior,
probe overhead, realtime WCET, driver/NIC attribution, and long-duration HIL.
