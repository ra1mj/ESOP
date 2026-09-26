# Design: DC offset and propagation delay programming

## Architecture

Add `DcClockController<const MAX_SLAVES: usize>` beside the existing
single-station `DcController` in `dc.rs`. The new controller owns a bounded
plan and a two-action sequence per System-Time-capable slave; it does not own
SYNC0/SYNC1 configuration or periodic FRMW synchronization.

```text
completed DcTopology + application/monotonic sample
  -> validate selected reference and every planned delay
  -> FPRD station 0x0910 / 24, exact WKC 1
  -> calculate corrected raw offset
  -> FPWR station 0x0920 / 12, exact WKC 1
  -> repeat in topology order
  -> publish complete immutable batch
```

`DcTopologySlave` gains the scanned `EscDcRange`. This keeps the clock-width
decision in the immutable Startup publication instead of coupling the
configuration controller back to mutable scan records.

## Controller Contract

`DcClockConfig` carries operation and per-request timeouts. `start()` accepts
the config, immutable topology, generation, application-time sample, and the
monotonic timestamp paired with that sample.

The controller copies eligible entries into `[DcClockPlanEntry; MAX_SLAVES]`.
It validates the whole candidate before emitting an action:

- a selected reference exists when at least one clock is planned;
- the selected reference is a planned System-Time-capable slave;
- every planned slave has cumulative transmission delay;
- timeout arithmetic and capacity are valid.

No-clock and empty topologies transition directly to Complete with an empty
published slice.

## State Machine

Phases are `Idle`, `ReadingClock`, `WritingOffsetDelay`, `Complete`, and
`Faulted`.

`ReadingClock` emits an exact 24-byte fixed read from `0x0910`. Acceptance
requires current generation, matching action identity, payload length 24,
exact WKC 1, and unexpired request/operation deadlines. Bytes `0..8` are the
sampled System Time; bytes `16..24` are the prior System Time Offset.

`WritingOffsetDelay` emits an exact 12-byte fixed write from `0x0920`:

```text
bytes 0..8   corrected System Time Offset, little endian
bytes 8..12  validated propagation delay, little endian
```

The current plan index advances only after the write terminal succeeds. The
last successful write transitions to Complete and makes the staged evidence
public.

## Time And Offset Math

At read acceptance:

1. `elapsed = response_now - start_monotonic` with checked regression.
2. `target = base_application_time + elapsed` with checked overflow.
3. For `Bits64`, calculate the signed difference in a wider integer and reject
   values outside `i64`.
4. For `Bits32`, calculate `target_low.wrapping_sub(system_low)` and interpret
   the result as an `i32`, yielding the shortest signed wrap-aware delta.
5. Add the signed delta to the old raw `u64` offset with wrapping two's-
   complement semantics.

All values remain integer nanoseconds. The controller does not read host wall
time and does not infer time quality.

## Actions And Evidence

Use a dedicated `DcClockAction` with a 24-byte fixed payload capacity so the
existing SYNC action ABI remains unchanged. Action matching includes kind,
station, register, length, generation, expected WKC, and write bytes.

`DcClockProgrammedSlave` records position, station, range, system-time sample,
target application time, old/new raw offset, and delay. The controller retains
bounded diagnostic `completed_count`, but `programmed_slaves()` returns the
full slice only in Complete. A fault clears the public length while retaining
typed fault/progress information.

## Production Integration

Extend `ScheduledProductionServiceKind` with `DcClockConfiguration` before
the existing `DcConfiguration` priority. Preserve the current scheduler
constructor and add `with_dc_clock_configuration(&mut controller)` so existing
call sites remain source-compatible.

`StartupConfigurationServices` gains a distinct requirement bit and builder.
The scheduler releases Startup only when every configured service, including
the new clock service when required, is Complete. Missing or faulted owners use
the existing fail-closed service error path.

The lifecycle guard maps the new service to `EthercatGate::Configuration`, the
same gate as mapping and existing DC configuration. The scheduler remains the
sole control-request producer; generic RX/control code remains register-
agnostic.

## Failure And Publication Semantics

Typed errors distinguish invalid configuration/topology, missing reference or
delay, generation/action/length/WKC mismatch, monotonic regression, arithmetic
overflow, request timeout, and operation timeout.

ESC writes accepted before a later fault are irreversible hardware side
effects. Therefore:

- complete batch evidence is not published until all writes succeed;
- `completed_count` reports bounded physical progress for diagnosis;
- fault/restart clears published batch evidence;
- no API calls the operation a rollback or atomic hardware transaction.

## Compatibility

- Existing `DcController`, `DcCyclicSync`, and scheduler constructor remain
  available with unchanged responsibilities.
- The topology range field and new service APIs are additive.
- Products must explicitly require/attach the clock service; legacy products
  continue through their existing configuration path.
- No ProcBuf, protobuf, cfggen schema, generated artifact, or cyclic frame-plan
  ABI change is required.

## Verification

Unit tests exercise the controller state machine and arithmetic boundaries.
Production-service tests prove ordering and barrier behavior. A public
request/RX test proves 24-byte read and 12-byte write ownership through the
existing pool. Lifecycle tests prove the new service is configuration-gated.
Repository checks cover `no_std`, feature matrices, documentation contracts,
BPF builds, and exact pushed-SHA GitHub Actions.
