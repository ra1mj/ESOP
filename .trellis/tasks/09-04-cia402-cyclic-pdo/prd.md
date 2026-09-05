# CiA 402 cyclic PDO binding

## Goal

Bind standard CiA 402 cyclic objects to fixed EtherCAT PDO images and enforce mode-specific, operation-enabled, permit, seed, and limit gates.

## Requirements

- Add a no-heap, fixed-layout PDO adapter for the standard CiA 402 objects used by the robot baseline.
- Require and validate `0x6040` Controlword, `0x6060` Modes of operation, `0x6041` Statusword, `0x6061` Modes of operation display, and `0x603F` Error code with the correct direction, width, signedness, and object identity.
- Support mode-specific cyclic objects: CSP `0x607A`/`0x6064`, CSV `0x60FF`/`0x606C`, and CST `0x6071`/`0x6077`. Support optional `0x60F4` Following error.
- Decode all configured TxPDO values into typed raw CiA 402 input records without heap allocation or text conversion.
- Write mode/control fields during the bounded enable and mode handshaking phase without requiring an active motion permit.
- Write a cyclic target only when lifecycle permit, confirmed mode, Operation Enabled state, and setpoint validation are all true. Reject an active cyclic write with any missing gate or a non-Operation-Enabled Controlword.
- Validate every field and image bound before mutating an output image. A rejected write must leave the caller-owned image unchanged.
- Keep scaling, vendor quirks, ESI/XML parsing, transport scheduling, and hardware-in-the-loop claims outside this task.

## Acceptance Criteria

- [x] A complete map validates for CSP, CSV, and CST and rejects missing or malformed common/mode-specific fields.
- [x] Input decoding round-trips signed position, velocity, torque, mode, statusword, error code, and following error values.
- [x] Output encoding uses the correct object for each mode and preserves unrelated bits in the process image.
- [x] Invalid direction/index/width/signedness, short images, denied gates, and non-operation-enabled controlwords are rejected without partial writes.
- [x] The adapter remains `no_std`, uses the existing core `PdoEntry` implementation, and has public regression tests.
- [x] `make ci` passes.

## Notes

- The profile crate remains the owner of CiA 402 semantics; the optional EtherCAT-facing adapter is the integration boundary for core PDO entries.
- A control-phase write is intentionally separate from a cyclic target write so the normal PDS enable sequence can complete without weakening the active-motion gate.
