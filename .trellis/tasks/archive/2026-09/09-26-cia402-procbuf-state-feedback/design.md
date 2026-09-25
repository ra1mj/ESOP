# Design: CiA 402 Feedback to ProcBuf State

## Context

The downlink path now turns ProcBuf targets into bounded CiA 402 PDO output and transmits them through the lifecycle-controlled EtherCAT cycle. The inverse path is incomplete: `Cia402PdoMap::read_inputs_for` can decode actual drive data, but `StopCycleContext` publishes a `StatePage` whose axes are not refreshed from the current process image. Consumers therefore cannot close the loop or distinguish current input from retained values.

This change completes that path while preserving the real-time and fail-closed properties of the cyclic core.

## Contract Changes

### ProcBuf ABI v6

`JointState` gains `error_code: u16`. The shared-memory layout hash changes and `ABI_VERSION` advances from 5 to 6. `ProcBuf::attach` continues to reject mismatched versions and layouts, so v5 regions must be recreated instead of migrated in place.

The remaining `JointState` fields keep their meaning. `quality` becomes a documented bitset:

| Bit | Name | Meaning when set |
| --- | --- | --- |
| 0 | `CURRENT_INPUT` | Domain/WKC evidence verifies this cycle's input image |
| 1 | `POSITION_VALID` | `position` was read and converted this cycle |
| 2 | `VELOCITY_VALID` | `velocity` was read and converted this cycle |
| 3 | `TORQUE_VALID` | `torque` was read and converted this cycle |
| 4 | `FOLLOWING_ERROR_VALID` | `following_error` was read and converted this cycle |
| 5 | `MODE_CONFIRMED` | actual mode equals the configured mode |
| 6 | `OPERATION_ENABLED` | statusword decodes to Operation Enabled |
| 7 | `FAULT_FREE` | statusword is not Fault/Fault Reaction Active and error code is zero |

When `CURRENT_INPUT` is clear, all other bits are also cleared. Values remain available for diagnostics but are explicitly stale.

### Protobuf v1

Add `uint32 drive_error_code = 11` to `esop.v1.JointState`. Existing field numbers and wire encodings stay unchanged. Frozen old fixtures must still decode, with the new field defaulting to zero.

## Data Flow

1. EtherCAT receive returns current `DomainInput` plus quality evidence.
2. The lifecycle guard validates domain identity, cycle freshness, and WKC through the existing quality projection.
3. For verified current input, each axis map decodes `Cia402PdoInputs` from the fixed process image.
4. A lifecycle helper copies the previous `[JointState; AXES]` into fixed staging storage and updates the staged entries:
   - common status/mode/state/error fields are replaced;
   - each mapped optional actual value is inverse-converted and marked valid;
   - each unmapped optional value is retained and its validity bit remains clear.
5. The helper applies accepted output controlwords to the same staged snapshot.
6. Only after every axis succeeds is `state.axes` replaced in one assignment.
7. `ProcBuf::publish_state` performs the existing double-page publication.
8. IPC/Zenoh readers project the v6 State data into the additive Protobuf field.

## Feedback Conversion

The immutable `Cia402AxisCommandPolicy` is reused as the bidirectional scale contract. This prevents command and feedback paths from silently using different scale or sign conventions.

For finite nonzero scale values:

```text
position       = (raw_position - position_offset_counts) / position_scale
velocity       = raw_velocity / velocity_scale
torque         = raw_torque / torque_scale
following_error = raw_following_error / position_scale
```

The conversion checks policy validity and finite output. Following error is a delta, so it does not remove the absolute position offset.

## Quality and Retention Semantics

The existing State snapshot is the retention store; no separate history allocation is required.

- Verified current input: update common fields, update mapped values, and construct quality bits from this cycle.
- Invalid/stale Domain or WKC evidence: keep all previous values and clear all per-axis quality bits.
- Missing optional mapped value: keep only that previous value and leave its validity bit clear.
- Current valid image with invalid map/policy/conversion: return a typed error before assigning staged axes. This is configuration/programming failure, not a stale-input condition.

Global State cyclic quality remains projected through the existing `CyclicQuality` flags. Axis quality provides the field-level contract that the global bitset cannot express.

## Accepted Output Tracking

The stop-cycle implementation will retain a fixed `[Cia402Output; AXES]` candidate for each output attempt and record it only after `transmit_outputs` succeeds.

- Accepted active frame: publish active controlwords.
- Active frame rejected, accepted fallback stop frame: publish fallback controlwords.
- All output attempts rejected: preserve prior State controlwords.
- Accepted active frame followed by a deadline miss: publish the active controlwords because the stop request has not yet reached the wire.

This keeps telemetry aligned with transport evidence rather than requested intent.

## Timestamp Semantics

`monotonic_time_ns` records the State publication decision time. `ecat_time_ns` advances from `DcCyclicSync::last_reference_time_ns()` only when current-cycle DC evidence is valid; otherwise the prior EtherCAT timestamp is retained. Existing State quality flags identify whether the current snapshot has valid DC evidence.

## Error Model

Add a typed lifecycle feedback error that preserves axis context and source category:

- invalid/nonzero-scale policy;
- PDO map validation/read failure;
- non-finite or unrepresentable inverse conversion.

`StopCycleError` wraps this feedback error. Errors occur before the staged axes are committed. Existing lifecycle stop/error handling remains responsible for fail-closed behavior.

## Compatibility and Rollout

1. Build all workspace binaries against ABI v6.
2. Reject old ProcBuf regions at attach time.
3. Recreate shared-memory regions during deployment restart.
4. Older Protobuf readers ignore field 11; newer readers accept old payloads with zero error code.
5. Rollback requires running the prior binaries with a recreated v5 region; v6 memory is never interpreted as v5.

## Verification Strategy

### Unit tests

- all CiA 402 modes and status decoding;
- signed inverse scale and position offset removal;
- following-error delta conversion;
- optional-field quality bits;
- stale-input retention;
- typed invalid policy/map/conversion failures;
- transactional multi-axis non-mutation.

### Integration tests

- simulated Linux EtherCAT receive to ProcBuf State publication;
- accepted active and fallback stop controlwords;
- rejected output retaining prior controlword;
- DC timestamp update and retention;
- IPC/Protobuf round trip including drive error code;
- frozen legacy Protobuf fixture compatibility.

### Regression gates

Run workspace formatting, lint/type checks, tests, BPF/eBPF build/test jobs, and documentation/capability consistency checks before commit and after push through GitHub Actions.
