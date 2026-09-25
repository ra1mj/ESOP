# Design: ProcBuf IPC Payload Adapter

## Ownership

`esop-ipc` owns hosted IPC framing and gains an optional `payloads` feature. That feature depends on `esop-procbuf`, `esop-proto`, and `esop-command-gateway`. The dependency direction remains host adapter to fixed/no-std contracts; no real-time crate depends on IPC or Protobuf.

The shared module owns:

- a bounded robot identity/configuration value,
- ProcBuf state and event projection into Protobuf-backed `IpcFrame` values,
- command-frame decoding and envelope/payload cross-checks,
- optional submission to `CommandIngress` after all structural checks pass.

Zenoh keeps transport/session concerns but imports these shared payload functions. Existing Zenoh-facing types may be re-exported for source compatibility.

## Payload Identity

The adapter configuration binds a textual Protobuf robot ID to the numeric ProcBuf/IPC robot ID, expected boot ID, expected layout hash, and fixed host source ID. Construction rejects empty/oversized/invalid identifiers and zero identity fields.

State frame mapping:

- `kind`: `State`
- `payload_schema_version`: Protobuf current schema
- `layout_hash`, `robot_id`, `boot_id`: validated ProcBuf header
- `source_id`: configured host adapter source
- `sequence`: state sequence
- `monotonic_time_ns`: state monotonic timestamp
- `quality_bits`: low 16 bits are `known_mask`; next 16 bits are `good_mask`
- payload: `RobotState`

Event frame mapping uses `Event`, the event sequence/timestamp, the configured host source in the envelope, and keeps the event's module source in `DiagnosticEvent.source`.

Command frames must use the configured numeric robot ID/layout hash and Protobuf schema. Envelope boot/source/sequence must equal the decoded command fields. Textual robot ID must equal the configured Protobuf identity. Authentication, when present, must equal the already-cross-checked command source.

## Projection State

The shared `ProcBufProjector` retains its existing single-reader replay floor. It validates the ProcBuf header before reading, advances the state floor only according to the existing read/projection contract, checks quality masks and lifecycle/stop consistency, rejects non-finite joint values, and emits a bounded message.

Protobuf messages are size-checked with `encoded_len()` before encoding. The IPC frame then performs its own payload-bound validation and CRC construction when serialized.

## Command Admission

Command processing is split into two explicit steps:

1. `decode_command_frame` performs all transport, schema, identity, and field-width checks and returns `ExternalMotionCommand`.
2. `admit_command_frame` calls `CommandIngress::admit` only after step 1 succeeds.

This ordering is required so malformed envelopes cannot consume replay sequence numbers, rate-limit budget, or audit slots. The existing ingress remains the only policy engine.

## Error Model

Use typed enums that preserve source errors from IPC framing, Protobuf decoding/schema validation, ProcBuf reads/projection, and command ingress. Identity mismatch variants distinguish robot, boot, source, sequence, layout, message kind, and authenticated-source failures for operational diagnosis without parsing text.

## Compatibility

The base `esop-ipc` crate keeps its current default dependency surface. The adapter is enabled by the `payloads` feature. The Zenoh `zenoh` feature enables `esop-ipc/payloads` and delegates to the shared implementation.

## Verification

Unit tests cover every identity mismatch and quality mapping. Integration tests create a ProcBuf, publish representative state/event records, transmit generated frames over real Unix datagram endpoints, decode them, and compare Protobuf fields. A command integration test transmits a command frame and proves admission returns a permit; a mismatch test snapshots ingress audit/state before and after rejection.
