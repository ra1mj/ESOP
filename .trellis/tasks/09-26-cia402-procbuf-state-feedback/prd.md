# Publish CiA 402 Feedback into ProcBuf State

## Goal

Complete the CiA 402 closed-loop state path required by FR-028 and ROB-003: after each EtherCAT cycle, project verified drive feedback and the controlword that was actually accepted by the transport into the ProcBuf `StatePage`, then expose the same data through the versioned IPC/Protobuf contract.

The result must let local and remote consumers distinguish current verified feedback from retained stale values without adding allocation, blocking, or non-deterministic work to the cyclic path.

## Requirements

### Functional requirements

1. The cyclic lifecycle path shall read CiA 402 input PDO values for every configured axis after a successful EtherCAT receive/domain/WKC cycle.
2. State projection shall publish, per axis:
   - actual position, velocity, torque, and following error when mapped;
   - statusword, actual mode, decoded drive state, and drive error code;
   - the controlword from the most recent output frame that the transport accepted.
3. Raw feedback shall be converted back to SI/application units with the same immutable axis policy used for command conversion:
   - position removes the configured offset and divides by the signed position scale;
   - velocity and torque divide by their signed scales;
   - following error divides by the signed position scale without applying the position offset.
4. Each `JointState` shall carry stable quality flags indicating:
   - current verified EtherCAT input;
   - validity of each optional actual-value field;
   - actual-mode confirmation;
   - operation-enabled state;
   - fault-free state.
5. If EtherCAT input evidence is missing, stale, or has invalid WKC/domain quality, the last valid numeric/status/error values shall be retained while current-quality flags are cleared. Stale values must never be presented as current.
6. If an optional feedback PDO is not mapped, its prior numeric value shall be retained and its corresponding validity flag shall be clear.
7. State mutation across all axes shall be transactional: conversion or map failure on one axis must not partially publish another axis's new feedback.
8. The State page shall expose the current monotonic publication time and update EtherCAT time only when current-cycle DC evidence is valid.
9. The IPC/Protobuf API shall expose the drive error code with an additive field that remains compatible with previously encoded v1 messages.
10. ProcBuf shall use ABI version 6 because the shared-memory `JointState` layout changes. Processes or regions using ABI versions 1 through 5 shall be rejected instead of reinterpreted.

### Safety and lifecycle requirements

1. The state `controlword` shall only change after the corresponding active or stop output frame is accepted by the EtherCAT transport.
2. If active output fails and a fallback stop output succeeds, State shall publish the accepted fallback stop controlword.
3. If no output frame is accepted, State shall retain the prior controlword.
4. A deadline miss detected after a successfully transmitted active frame shall preserve that accepted active controlword while lifecycle state records the stop request; the system shall not claim that an unsent stop frame was applied.
5. Valid current feedback containing a nonzero drive error or fault state shall remain observable; projection must not erase fault evidence.
6. Invalid axis scale policy, out-of-range conversion, or malformed current PDO mapping shall produce a typed error and no partial State-axis update.

### Real-time constraints

1. The cyclic path shall remain `no_std`, allocation-free, bounded by `AXES`, and free of blocking synchronization.
2. Conversion shall use fixed-size staging storage and a single assignment to publish the completed axis snapshot.
3. No new filesystem, network, logging, formatting, or heap work may be introduced into the real-time cycle.

### Documentation and capability requirements

1. The software PRD, robotics implementation plan, IPC contract, lifecycle documentation, README, and capability manifest shall describe ProcBuf ABI v6 and the feedback behavior.
2. The capability manifest shall add an implemented `cia402_procbuf_feedback` capability while retaining explicit limitations for physical HIL, product-scale axes, and WCET certification.
3. Migration notes shall state that ABI v5 shared-memory regions must be recreated before using ABI v6 binaries.

## Acceptance Criteria

- [x] `JointState` includes a drive error code and ProcBuf reports ABI version 6 with a changed layout hash.
- [x] Protobuf `JointState` includes additive field 11 for the drive error code; legacy fixture decoding remains unchanged and current round trips include the new field.
- [x] Verified EtherCAT input updates every mapped feedback field, statusword, actual mode, decoded drive state, drive error code, and quality flags for all axes.
- [x] Signed-scale and position-offset inverse conversions are covered by deterministic unit tests.
- [x] Invalid/stale domain or WKC evidence retains prior values and clears current/field quality flags.
- [x] Missing optional PDOs retain prior values and clear only the corresponding validity flags.
- [x] Multi-axis conversion failure leaves the entire prior axis snapshot unchanged and returns a typed error.
- [x] State `controlword` reflects accepted active output, accepted fallback stop output, or the prior value when no output was accepted.
- [x] Current-cycle DC evidence updates `ecat_time_ns`; invalid DC evidence does not invent a new EtherCAT timestamp.
- [x] Linux simulated end-to-end tests cover PDO receive, output acceptance, ProcBuf publication, and IPC/Protobuf projection.
- [x] Existing lifecycle, EtherCAT, ProcBuf, IPC, BPF/eBPF, and Zenoh tests remain green.
- [x] Documentation and `capability_manifest.json` consistently report ABI v6 and the implemented feedback capability.
- [ ] Changes are committed, pushed to `ra1mj/ESOP`, and the resulting GitHub Actions run succeeds.

## Out of Scope

- Physical EtherCAT hardware validation or servo tuning.
- Product-scale axis-count expansion beyond the current fixed `AXES` contract.
- Certified WCET measurement.
- Automatic fault reset or diagnostic interpretation beyond preserving raw CiA 402 fault evidence.
- Redesign of the EtherCAT process-image mapping format.
