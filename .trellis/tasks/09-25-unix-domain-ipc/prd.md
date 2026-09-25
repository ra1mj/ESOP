# Implement bounded Unix domain IPC transport

## Goal

Implement the first controlled FR-029 IPC transport for the Linux supervision
domain. The transport must carry a bounded, versioned envelope with robot,
layout, boot, source, sequence, monotonic-time and quality identity; detect
replay, stale messages, peer restart and peer loss; and never introduce POSIX
or transport dependencies into the real-time EtherCAT, lifecycle, profile or
ProcBuf crates.

## Background

- `esop-procbuf` already owns the fixed real-time ABI, including layout hash,
  robot/boot identity, command/state sequence, timestamps and quality facts.
- `esop-command-gateway` already owns external motion-command authorization,
  TTL, replay, rate-limit and audit policy. IPC must not duplicate that policy.
- The workspace has no `esop-ipc` implementation. The product plan names Unix
  domain sockets, shared memory and RPMsg as allowed IPC mechanisms, and FR-029
  requires every declared mechanism to expose version, sequence, timestamp,
  quality and disconnect/restart behavior.
- A Unix datagram transport provides message boundaries and bounded
  nonblocking sends without stream reassembly. Peer liveness must still be
  derived from explicit heartbeat/data observations rather than socket reads
  blocking or implicitly keeping motion alive.

## Requirements

1. Add a host-only `esop-ipc` crate with one authoritative fixed-capacity frame
   codec and one Linux/Unix datagram endpoint. Real-time crates must not depend
   on it.
2. Every frame shall carry protocol/header version, message kind, payload
   schema version, layout hash, robot ID, boot ID, source ID, sequence,
   monotonic timestamp, quality bits, payload length and payload integrity.
3. Encoding and decoding shall use an explicit byte order and field offsets;
   raw Rust struct layout, padding and `transmute` are forbidden.
4. Frames shall have a compile-time payload maximum. Oversized, truncated,
   corrupt, unknown-kind, reserved-field, zero-identity and zero-sequence
   inputs must be rejected before delivery to a consumer.
5. The endpoint shall be nonblocking. Empty receive, local send pressure,
   absent peer, unexpected source path and oversized datagrams must be
   distinguishable without sleeping, retry loops or background threads.
6. A single peer monitor shall validate the configured robot/layout/source,
   reject replay and same-boot timestamp regression, enforce maximum transport
   age, detect boot changes as restart, detect timeout as offline, and report a
   valid same-boot return after timeout as reconnect.
7. Socket-path ownership shall be explicit: binding must not remove an
   existing path, and dropping a successfully bound endpoint shall remove only
   the path it owns.
8. Integration tests shall use real Unix datagram sockets and cover
   bidirectional transfer, no-data behavior, source rejection, peer absence,
   endpoint restart on the same path, boot change, timeout and reconnect.
9. Update the software PRD, software plan, capability manifest and a dedicated
   IPC contract document. Claims must remain limited to hosted Unix datagram
   behavior; shared memory, RPMsg, payload-specific adapters and production
   deployment qualification remain open.

## Acceptance Criteria

- [x] A valid maximum-size frame round-trips every header field and payload
      byte through the codec; the frozen field offsets and byte order are
      covered by golden-byte tests.
- [x] Malformed header length/version, unknown kind, nonzero reserved value,
      zero required identity/sequence, oversized/truncated payload and CRC
      corruption are rejected with stable typed errors.
- [x] Two real nonblocking Unix datagram endpoints exchange command, state and
      heartbeat frames in both directions without partial delivery or retry
      loops; an empty socket returns `WouldBlock`.
- [x] A datagram from an unexpected socket path, a missing peer and a datagram
      exceeding the fixed receive capacity are rejected distinctly.
- [x] Peer monitoring rejects robot/layout/source mismatch, replay, timestamp
      regression, future/stale frames and backwards local time; it reports
      first contact, continued contact, restart, offline and reconnect.
- [x] Dropping and rebinding a peer at the same path leaves no stale local
      socket file and a new boot is observed as a restart rather than replay.
- [x] `esop-ethercat-core`, `esop-lifecycle-guard`, `esop-profile-cia402` and
      `esop-procbuf` dependency trees contain no `esop-ipc`, POSIX or socket
      dependency; existing no-std checks remain green.
- [x] Focused tests/Clippy, workspace `make ci`, formatting, capability
      validation and `git diff --check` pass.
- [ ] Changes are committed and pushed to `ra1mj/ESOP`; the matching final
      GitHub Actions run succeeds before task archive.

## Out Of Scope

- Shared-memory and RPMsg transports.
- Direct serialization of ProcBuf Rust structs or payload-specific
  command/state/event adapters.
- Cryptographic peer identity, SELinux/AppArmor policy and deployment ACLs.
- Windows named pipes, TCP, remote networking, ROS 2 and Zenoh integration.
- Production latency/WCET, long-duration stress, target MCU or hardware HIL
  qualification.
