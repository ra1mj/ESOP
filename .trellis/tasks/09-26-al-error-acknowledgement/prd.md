# Implement AL error acknowledgement and Device Emulation safety

## Goal

Close the EtherCAT PRD P0 gap for AL Error Indication acknowledgement while
preserving fail-closed startup behavior and preventing an acknowledge write to
Device Emulation slaves.

## Background

- The scanner already discovers ESC identity/capacity data, assigns fixed
  station addresses, and reads AL Status plus AL Status Code.
- `AlTransitionController` currently latches any AL error immediately. It does
  not acknowledge the slave, distinguish Device Emulation, or retain an
  acknowledgement result.
- `StartupController` retains topology and identity across its PREOP
  configuration barrier, but its terminal AL failure only exposes the protocol
  error and code without a complete per-slave fault record.
- Beckhoff EtherCAT Slave Controller documentation defines Device Emulation at
  `0x0141[0]`, Error Indication at `0x0130[4]`, and Error Indication
  Acknowledge at `0x0120[4]`. When Device Emulation is enabled, AL Control is
  copied into AL Status, so the acknowledge bit must never be written.

## Requirements

### R1. Discover Device Emulation

1. During the bounded online scan, read ESC Configuration register `0x0141`
   for every discovered slave after assigning its fixed station address.
2. Persist bit 0 as a typed `device_emulation` capability associated with that
   scan record and preserve it through startup activation.
3. The extra request shall use the existing control pool, generation, WKC, and
   deadline checks. A missing, short, stale, or timed-out response shall fault
   scan/startup rather than defaulting the capability.

### R2. Explicit AL acknowledgement policy

1. Preserve existing direct `AlTransitionController::start` behavior for
   callers that do not opt into acknowledgement.
2. Add an explicit policy-aware start path that accepts the already observed
   AL status and enables acknowledgement only for non-Device-Emulation slaves.
3. When an enabled controller observes Error Indication, retain the first
   error state and AL Status Code, write the observed state plus the Error
   Indication Acknowledge bit, and poll AL Status until the bit clears or the
   original transition deadline expires.
4. A successful acknowledgement clears the slave indication only. The
   controller shall still enter `Faulted` with the original AL error code and
   shall not retry the requested transition automatically.
5. If acknowledgement times out or its response is invalid, retain the first
   AL fault record while reporting the terminal transport/protocol failure.

### R3. Device Emulation fail-closed behavior

1. Startup shall disable acknowledgement when the scanned capability reports
   Device Emulation.
2. An AL error reported for a Device Emulation slave shall fault immediately.
3. No action produced for a Device Emulation slave may contain AL Control bit
   4, including initial-error, transition-error, retry, and restart paths.

### R4. Diagnostic evidence

1. Expose a fixed-size AL fault record containing the first observed AL status,
   requested state, and acknowledgement result.
2. Startup shall expose the affected position, fixed station address,
   requested state, actual state, AL Status Code, Device Emulation capability,
   and acknowledgement result after a terminal AL failure.
3. Restarting startup shall clear prior AL fault evidence and rediscover the
   capability instead of reusing stale state.

### R5. Compatibility and bounded execution

1. Preserve `no_std`, allocation-free, fixed-capacity, caller-driven behavior.
2. Do not add sleeps, threads, dynamic dispatch, heap storage, or unbounded
   retries.
3. Keep the current request/action identity and production scheduler ownership
   model unchanged.
4. Existing callers that do not use the new policy-aware API shall remain
   source-compatible and retain immediate fault latching.

### R6. Qualification boundary

1. Software tests prove register sequencing, fault retention, and fail-closed
   lifecycle projection only.
2. Physical ESC behavior, interoperability, target WCET, full-period WKC,
   functional-safety qualification, and HIL remain outside this task.

## Acceptance Criteria

- [x] Scan reads `0x0141`, records both Device Emulation values, and faults on
      invalid capability responses.
- [x] A normal slave with an initial or transition-time AL error receives one
      bounded acknowledge write using the observed state plus bit 4, is polled
      until clear, then remains faulted with the original status code.
- [x] A Device Emulation slave with the same injected status faults without any
      AL Control write containing bit 4.
- [x] Acknowledgement timeout/invalid response preserves the original slave,
      requested/actual state, code, capability, and acknowledgement stage.
- [x] Explicit startup restart clears old evidence and recomputes policy from a
      new scan.
- [x] Core unit tests exercise normal and Device Emulation sequences; Linux
      simulated-port coverage exercises the added scan request through the
      shared production control path.
- [x] Existing startup, production scheduling, product configuration, `no_std`,
      Clippy, CI, BPF, HIL harness, and Zenoh test gates pass.
- [x] PRD, EtherCAT requirements, capability manifest, README/status text, and
      Trellis specs describe the delivered boundary without claiming physical
      qualification.

## Out of Scope

- Automatic retry of a failed AL transition after acknowledgement.
- ESI/SII transition timeout discovery or ETG.1020 timeout defaults.
- `OpOnly` output SyncManager isolation.
- Mailbox/SM/FMMU/DC descriptor discovery and generated configuration batches.
- Hardware certification, conformance, or functional-safety claims.
