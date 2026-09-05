# Technical Design

## Boundary

The adapter lives in `crates/esop-profile-cia402/src/pdo.rs` and is exported from the profile crate. It depends on `esop-ethercat-core` only for the existing `PdoEntry`, `PdoError`, and `PdoLayout` contracts. The profile's FSA and mode supervisor remain usable without transport calls; the adapter only translates typed profile values to and from a caller-owned process image.

The process image passed to the adapter may be a direction-local image or a previously generated unified image. `PdoEntry.bit_offset` is authoritative. No network, mailbox, scaling, or vendor-specific behavior is introduced here.

## Data Flow

```text
static PdoEntry map
  -> validate object identity/direction/width/signedness/overlap
  -> read Tx image -> Cia402PdoInputs
  -> controller/mode/MLG/setpoint decisions
  -> Cia402PdoCommand + Cia402MotionGate
  -> validate all output fields and bounds
  -> write Rx image
```

## Contracts

- `Cia402PdoField` identifies each supported standard object.
- `Cia402PdoMap` stores one optional `PdoEntry` per field in a fixed array and exposes `validate_for(mode)` and `validate_all_modes()`.
- `Cia402PdoInputs` contains raw integer values and `OperatingMode`; unknown mode bytes remain `OperatingMode::Unknown` so the existing supervisor can fail closed.
- `Cia402Target` makes the target width/type explicit for position, velocity, and torque.
- `write_control` writes only Controlword and Modes of operation for the enable/mode handshake.
- `write_cyclic` additionally writes the mode-specific target and requires `Cia402MotionGate::allows_motion()` plus the Controlword Operation Enabled low nibble.

## Validation and Atomicity

Validation is split into common-field, mode-field, and present-entry checks. All entries are added to a fixed `PdoLayout` to reject same-direction overlap and duplicate objects. Before any write, every destination entry is read once to validate the image bounds; typed values have exact standard widths, so a successful preflight makes the subsequent bounded writes non-failing. Any validation error is returned before the first mutation.

The adapter never allocates, waits, parses text, or changes a frame plan. It is safe to call from the already-activated cycle owner when the caller supplies the frozen map and image.

## Compatibility

Existing profile APIs and the core crate remain source-compatible. The new dependency is one-way (`profile -> core`); core does not depend on profile. Vendor scaling and quirks can later wrap this raw adapter without changing the EtherCAT core.
