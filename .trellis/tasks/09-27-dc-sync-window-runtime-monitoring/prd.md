# DC sync-window runtime monitoring

## Goal

Complete the next software-only FR-018/DC-003 increment by adding an optional
topology-wide runtime synchronization-window sample to the existing cyclic DC
exchange, exposing bounded lock/loss/recovery evidence, and making the motion
lifecycle DC gate depend on a fresh complete reference-clock plus sync-window
observation from the same cycle.

## Background

- `DcCyclicSync` already emits one cyclic FRMW request to the selected
  reference clock, compares returned reference time with caller-provided
  application time, and applies offset/jitter hysteresis.
- `ScheduledDomainBank` already submits the DC frame, shares one bounded RX
  pass with Domains and at most one control request, and finalizes missing DC
  responses fail-closed.
- Lifecycle and ProcBuf projections currently qualify DC only when the FRMW
  sample is current and the existing application/reference monitor is locked.
- Startup now verifies the immutable DC topology and the product-wide SYNC
  plan, but runtime `0x092c` synchronization-window observation remains an
  explicit documented gap.
- IgH queues a four-byte broadcast read of ESC `0x092c` and reports the lower
  31-bit synchronization difference magnitude. ESOP shall implement an
  independently authored fixed-capacity contract around the same observable
  register behavior.

## Requirements

1. Add a fixed-size `DcSyncWindowConfig` describing one BRD datagram index,
   process-image offset, exact nonzero expected WKC, maximum accepted
   difference, and nonzero lock/loss hysteresis windows.
2. Provide a checked helper that derives the exact BRD expected WKC from the
   immutable Startup `DcTopology` length; empty or `u16`-overflowing topology,
   a duplicate reference/window datagram index, zero thresholds, or invalid
   process-image bounds shall fail with typed errors before TX.
3. Keep `DcCyclicConfig::new` and `DcCyclicSync::new` behavior-compatible. A
   caller shall opt in through an additive constructor/builder; legacy callers
   shall continue to own one FRMW datagram and use the existing lock result.
4. When enabled, `DcCyclicSync` shall expose a second immutable datagram plan:
   a four-byte `Command::Brd` of ESC System Time Difference `0x092c` with a
   zeroed request payload and the configured exact WKC.
5. Reference and sync-window responses shall use distinct indices and the same
   RX generation. Command, address, length, generation, WKC, ownership, and
   process-image bounds shall be checked exactly.
6. Decode the synchronization difference as the lower 31-bit magnitude of the
   little-endian register value. Preserve the raw word for diagnostics without
   treating the sign bit as magnitude.
7. Stage both returned values locally and publish neither monitor sample until
   every enabled DC response for that generation succeeds. Response arrival
   order shall not affect the result.
8. A missing or rejected response shall close the pending generation, preserve
   the first typed error, publish no partial current-cycle evidence, and count
   as a bad sync-window observation when the optional monitor is enabled.
9. Add a fixed-size `DcSyncWindowMonitor` that exposes state, current maximum
   difference, last sample time, sample count, good/bad cycles, loss count, and
   recovery count. Consecutive good samples acquire/reacquire lock; consecutive
   bad or missing samples reach `Unlocked`; intermediate states remain
   distinguishable.
10. `DcCyclicSync::is_locked()` shall require the existing offset/jitter
    monitor and, when configured, the sync-window monitor to be `Locked`.
    Existing monitor accessors remain available for diagnostics.
11. The scheduled DC TX plan shall admit one or two DC datagrams, reject any
    Domain/DC/control index alias before mutation, and keep one frame and one
    bounded RX owner. A failed TX or RX shall finalize both DC observations.
12. Lifecycle and ProcBuf quality projection shall consume the combined lock
    predicate and still require a complete current-cycle DC sample. A retained
    previous window result shall never qualify a new cycle.
13. Public tests shall cover topology-derived WKC, exact BRD wire shape, both
    response orders, threshold/hysteresis, missing response, WKC/index/shape
    rejection, index conflicts, shared Domain/DC/control RX, lock loss, and
    recovery into the lifecycle DC gate.
14. Documentation and capability evidence shall state the implemented
    software boundary and shall not claim per-slave identity from the broadcast
    aggregate, periodic drift compensation, external application-time
    discipline, physical timing precision, HIL, ETG conformance, or functional
    safety qualification.

## Acceptance Criteria

- [x] Core unit tests prove checked configuration, topology WKC derivation,
      BRD `0x092c/4` plan shape, lower-31-bit decoding, atomic same-generation
      publication, response-order independence, missing/error handling,
      hysteresis, loss counting, and recovery counting.
- [x] Scheduled-domain tests prove the optional second DC datagram shares the
      normal frame/RX path, exact WKC is enforced, and reference/window indices
      cannot alias Domains or control requests.
- [x] Lifecycle and ProcBuf tests prove a current FRMW sample alone cannot
      qualify DC when sync-window monitoring is enabled, one bad/missing window
      sample blocks motion immediately, and a fresh configured good window
      reacquires the gate only through normal qualification.
- [x] A public Linux simulated-port test observes threshold crossing and
      recovery through the same generated cyclic path used by production
      service scheduling.
- [x] README, requirements/PRD/architecture docs, backend specs, and capability
      manifest describe runtime sync-window monitoring and its limitations.
- [ ] Focused tests, workspace all-feature checks/tests/Clippy, AArch64
      `no_std`, `make ci`, `make bpf`, Trellis validation, capability-manifest
      validation, and exact pushed-SHA GitHub Actions all pass.

## Out Of Scope

- Reading every slave individually or attributing the broadcast aggregate to
  one slave position.
- Automatically changing DC offset/delay or SYNC0/SYNC1 programming after a
  bad sample; monitoring remains evidence and the lifecycle owns fail-closed
  motion behavior.
- PTP/TAI/GNSS application-time discipline, periodic drift compensation, ring
  redundancy, hot connect, or online topology recalculation.
- Real ESC response provenance, physical oscillator accuracy, target WCET,
  drive interoperability, long-duration HIL, ETG conformance, or functional
  safety qualification.

## Technical Notes

- The BRD expected WKC is explicit and checked because ESOP's RX engine does
  not accept a transport response as quality evidence without an exact WKC.
- Sync-window hysteresis describes the register observation; lifecycle safety
  remains stricter and rejects any current-cycle non-locked or unavailable DC
  evidence immediately.
- The existing FRMW application/reference offset remains the ProcBuf
  `dc_offset_ns`. The broadcast maximum difference is exposed through the core
  API in this increment, avoiding an unrelated ProcBuf ABI change.

## Verification Evidence

Verified locally on 2026-09-27:

- `cargo test -p esop-ethercat-core --lib`: 247 passed.
- `cargo test -p esop-lifecycle-guard --all-features`: 53 passed.
- `cargo test -p esop-ethercat-linux-port --test scheduled_domains`: 19 passed.
- `cargo check --workspace --all-features`, workspace all-feature tests, and
  warnings-as-errors Clippy passed.
- `make no-std`, `make capability-manifest`, `make ci`, and `make bpf` passed.
- Trellis task validation, JSON validation, formatting, and `git diff --check`
  passed.
- Exact pushed-SHA GitHub Actions evidence is recorded after publication.
