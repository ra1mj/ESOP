# Technical design

## Boundaries

`esop-ipc` is a Linux/Unix supervision-domain crate. It has no dependency on
`esop-procbuf` or the real-time crates. Those crates remain the owners of the
RT ABI and safety policy; future host adapters may copy validated snapshots or
encoded payloads into this transport.

```text
ProcBuf / gateway adapter (future)
  -> IpcFrame::new + encode
  -> nonblocking UnixDatagram::send_to
  -> kernel datagram boundary
  -> recv_from + source-path check + decode
  -> PeerMonitor::observe
  -> payload-specific consumer (future)
```

Transport validation does not authorize motion. `CommandIngress` and the MLG
remain the only external-command and motion-permit policy owners.

## Crate layout

- `src/frame.rs`: fixed wire constants, `MessageKind`, `IpcHeader`,
  `IpcFrame`, encoder/decoder, CRC-32 and typed codec errors.
- `src/peer.rs`: `PeerPolicy`, `PeerMonitor`, status/observation enums and
  typed identity/replay/time errors.
- `src/unix.rs`: nonblocking `UnixDatagramEndpoint`, source-path validation,
  endpoint ownership/cleanup and transport error mapping.
- `src/lib.rs`: public exports and scope documentation.
- `tests/unix_loopback.rs`: kernel-backed bidirectional, restart, source and
  oversize integration coverage.

No external runtime dependency is required.

## Wire contract

The v1 header is exactly 80 bytes in little-endian order:

| Offset | Size | Field |
| ---: | ---: | --- |
| 0 | 4 | magic `ESIP` |
| 4 | 2 | protocol version |
| 6 | 2 | header bytes |
| 8 | 2 | message kind |
| 10 | 2 | reserved, must be zero |
| 12 | 4 | payload schema version |
| 16 | 8 | ProcBuf/layout hash |
| 24 | 8 | robot ID |
| 32 | 8 | boot ID |
| 40 | 8 | source/component ID |
| 48 | 8 | source sequence |
| 56 | 8 | source monotonic time ns |
| 64 | 8 | quality bits |
| 72 | 4 | payload length |
| 76 | 4 | IEEE CRC-32 of payload |

`MAX_PAYLOAD_BYTES` is 4096 and `MAX_DATAGRAM_BYTES` is 4176. The decoder
requires exact datagram length `HEADER_BYTES + payload_len`; no trailing bytes
or silent truncation are accepted. Stable message kinds are Command, State,
Event, Heartbeat and Diagnostic. Heartbeats have an empty payload; all other
kinds require a nonempty payload.

The header version governs field layout. Payload schema evolution is separate
and adapter-owned. V1 does not expose Rust memory layout or pointer-width
dependent values.

## Frame API

`IpcFrame` owns a fixed payload array and logical length. Construction validates
the complete header/payload contract. `encode_into` writes to a caller buffer;
`decode` returns an owned frame so no receive-buffer lifetime escapes.

CRC-32 is integrity detection only, not authentication. It catches truncation
and accidental corruption before the payload reaches an adapter.

## Peer state

`PeerPolicy` freezes expected robot ID, layout hash, source ID, offline timeout
and maximum message age. `PeerMonitor` stores only the last accepted boot,
sequence, remote timestamp and local receive time.

Observation order:

1. Validate configured identity and local time monotonicity.
2. Reject remote future time or age above the configured maximum.
3. For the same boot, require strictly increasing sequence and nondecreasing
   remote monotonic time.
4. A different nonzero boot is `Restarted` and resets replay floors.
5. A same-boot message after `offline_timeout_ns` is `Reconnected` if all
   replay/time checks pass.

`status(now_ns)` returns NeverSeen, Online or Offline. `WouldBlock` does not
change monitor state; only elapsed local monotonic time does.

## Unix endpoint

`UnixDatagramEndpoint::bind(local, peer)` binds a filesystem socket and sets it
nonblocking. It refuses an existing local path. `send` performs one encode and
one `send_to`; short writes are errors. `receive` uses a buffer one byte larger
than the maximum valid datagram so truncation/oversize is observable, validates
the sender pathname, then decodes.

The endpoint owns only a path it successfully bound. `Drop` removes that socket
path. It never removes a pre-existing path and never retries or sleeps.

## Errors

- Codec errors distinguish version/header/kind/reserved/identity/sequence,
  payload shape/length and CRC failures.
- Peer errors distinguish identity mismatch, replay, remote/local time
  regression, future timestamp and stale age.
- Unix errors distinguish would-block, absent/refused peer, unexpected source,
  oversized datagram, short write, codec rejection and other I/O errors.

Raw `std::io::Error` is retained for uncategorized OS failures, while stable
high-level variants cover behavior asserted by the contract.

## Compatibility and rollback

The crate is additive and initially has no production consumer. Removing it
restores the previous dependency graph without changing ProcBuf, MLG, EtherCAT,
Protobuf or Zenoh behavior. Future incompatible header changes require a new
protocol version; fields and offsets in v1 are frozen.

## Verification

- Unit tests freeze golden bytes, CRC and every validation class.
- Peer tests freeze all state transitions and time/replay boundaries.
- Unix integration tests use unique temporary paths and real kernel datagrams.
- Cargo-tree inspection proves RT crates do not acquire the IPC crate.
- Full `make ci` proves workspace and no-std behavior remain unchanged.
