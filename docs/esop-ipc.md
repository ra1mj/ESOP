# ESOP Hosted Unix Datagram IPC

`esop-ipc` implements the hosted portion of FR-029. It provides a bounded,
nonblocking filesystem Unix datagram transport for communication between Linux
supervision processes. The crate is deliberately outside the real-time
`no_std` dependency graph.

## Wire Contract

Protocol v1 uses one explicit 80-byte little-endian header followed by at most
4096 payload bytes. Rust layout and `transmute` are not part of the ABI.

| Offset | Type | Field |
| ---: | --- | --- |
| 0 | `u32` | ASCII magic `ESIP` |
| 4 | `u16` | protocol version |
| 6 | `u16` | header bytes |
| 8 | `u16` | message kind |
| 10 | `u16` | reserved, must be zero |
| 12 | `u32` | payload schema version |
| 16 | `u64` | layout hash |
| 24 | `u64` | robot ID |
| 32 | `u64` | boot ID |
| 40 | `u64` | source/component ID |
| 48 | `u64` | sequence |
| 56 | `u64` | source monotonic time in ns |
| 64 | `u64` | quality bits |
| 72 | `u32` | payload bytes |
| 76 | `u32` | IEEE CRC-32 of the payload |

Command, state, event and diagnostic frames require a nonempty payload.
Heartbeat frames require an empty payload. Schema version, layout hash, robot
ID, boot ID, source ID and sequence must be nonzero. Decoding requires the
datagram length to exactly match the declared payload length.

CRC detects accidental corruption; it is not authentication. The optional
`payloads` feature owns the current ProcBuf/Protobuf mapping. ProcBuf Rust
structs are never serialized from their memory layout.

## ProcBuf and Protobuf Payloads

With the `payloads` feature, `ProcBufProjector` is the sole host-side reader for
one ProcBuf state page and event ring. It validates the ABI header, layout hash,
numeric robot ID and boot ID before consuming data, then projects owned
`RobotState` and `DiagnosticEvent` messages for both IPC and Zenoh. The textual
robot ID is a separate fixed-capacity deployment identity using the same
64-byte identifier alphabet as the Zenoh namespace.

State frames bind the Protobuf and IPC identities as follows:

- frame schema, layout, numeric robot, boot, source, sequence and monotonic
  timestamp are populated explicitly;
- the low 16 quality bits carry ProcBuf `known_mask`, and bits 16-31 carry
  `good_mask`; stale quality carries zero, while malformed masks reject the
  snapshot;
- the Protobuf payload retains full lifecycle, per-axis stop, quality, joint
  and IO fields and remains bounded by 4096 bytes.

Event frames use the configured host adapter source in the envelope while the
event-producing module remains in `DiagnosticEvent.source`.

Command handling has two ordered stages. `decode_command_frame` requires a
Command frame and cross-checks envelope schema/layout/numeric robot/boot/source/
sequence against the configured identity and decoded `MotionCommand`.
`admit_command_frame` calls `CommandIngress` only after those checks and an
optional authenticated-source comparison succeed. Structural failures therefore
do not consume replay floors, rate-limit budget or audit slots. ACL, authority,
TTL, permit epoch, axis policy and motion permits remain owned by
`esop-command-gateway`.

## Peer Lifecycle

`PeerMonitor` binds accepted traffic to an expected robot ID, layout hash and
source ID. Within one boot ID it requires a strictly increasing sequence and a
nondecreasing remote monotonic timestamp. It rejects future and stale frames
against caller-provided local monotonic receive time.

A different boot ID produces `Restarted` and resets sequence/time floors. A
same-boot frame after the offline timeout produces `Reconnected`. Status is
`NeverSeen`, `Online` or `Offline`; failed observations never mutate monitor
state. Command authorization, motion permits and ACL decisions remain the
responsibility of `esop-command-gateway` and the lifecycle guard.

## Unix Endpoint

`UnixDatagramEndpoint` is nonblocking from bind onward. One call performs at
most one encode/send or one receive/decode operation; there are no retries,
sleeps or background threads. Receive admission requires the exact configured
filesystem peer path. The API distinguishes:

- local `WouldBlock` pressure;
- unavailable peer paths;
- unexpected source paths;
- oversized datagrams;
- short writes;
- typed frame codec errors;
- other OS I/O errors.

Binding refuses an existing local path and never unlinks it. On drop, an
endpoint removes the local socket only when its device/inode still matches the
socket created by that endpoint, so a replacement path is preserved.

Run focused transport and payload verification with:

```sh
make test-ipc
```

The integration tests use real kernel Unix datagram sockets for bidirectional
command/state/heartbeat traffic, projected ProcBuf state/event payloads, valid
command admission, envelope/payload mismatch rejection, empty nonblocking
receive, absent and unexpected peers, oversized datagrams, same-path peer
rebinding with a new boot ID, and owned-path cleanup.

## Claim Boundary

This implementation does not claim shared memory or RPMsg transport,
MotionCommand target conversion into a ProcBuf Command page, cryptographic peer
identity, SELinux or filesystem deployment policy, production latency/WCET,
long-duration stress, or target HIL qualification. Those remain separate
acceptance gates.
