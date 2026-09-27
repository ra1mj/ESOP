# Topology-wide DC SYNC programming

## Goal

Complete the next software-only FR-018 increment by turning each product's
verified ESI/SII DC mode into a bounded, ordered all-slave SYNC0/SYNC1 plan and
programming that plan around one common reference-clock epoch before Startup
may leave its PREOP configuration barrier.

## Background

- `DcClockController` already initializes System Time Offset and propagation
  delay for every System-Time-capable slave from the immutable Startup DC
  topology.
- Product generation already selects one exact ESI `Dc/OpMode` for each
  DC-required slave, and Startup verifies its SII Strings/DC descriptor before
  the first AL action.
- The existing `DcController` programs one caller-selected station and derives
  activation from a local enum. It does not consume product order, preserve the
  complete `AssignActivate` word, coordinate a shared epoch, or publish a
  topology-wide result.
- The production scheduler and Startup barrier already provide independent
  slots for clock initialization and legacy single-station DC configuration.
- The checked-in dual-axis product has a 1 ms base period and selects `DcSync`
  with `AssignActivate=0x0300` for both drives.

## Requirements

1. A single shared timing resolver shall convert the selected SII-representable
   DC mode and product base period into exact `u32` SYNC0/SYNC1 register values,
   signed SYNC0 shift, and the unchanged 16-bit `AssignActivate` word.
2. Direct nonzero SYNC0 cycle time shall take precedence over its factor.
   Otherwise positive factors multiply the product base period and negative
   factors divide it exactly; zero, inexact, overflowed, or out-of-range
   results shall fail with typed errors.
3. When `AssignActivate` enables SYNC1, its register interval shall use the
   established EtherCAT master relation based on SYNC0 cycle, product base
   period, signed factor, and signed SYNC1 shift. A mode that does not enable
   SYNC1 shall require zero SYNC1 factor and shift and shall program zero.
4. DC-required modes shall activate the DC unit and SYNC0. Unsupported or
   internally inconsistent activation/timing combinations shall fail during
   generation and again at the fixed-size runtime boundary.
5. Generated normalized JSON, device inventory, C, and Rust artifacts shall
   carry the resolved timing. All resolved values shall participate in the
   deterministic product configuration hash.
6. `StaticProductConfig` shall build a fixed-capacity `DcSyncPlan` in product
   slave order, include only DC-required slaves, preserve exact positions and
   station addresses, and identify the one configured reference clock.
7. Building Startup profiles shall validate that raw selected-mode metadata,
   resolved timing, DC requirement, and reference policy agree before mutating
   Startup.
8. A new topology-wide `DcSyncController<MAX_SLAVES>` shall validate the whole
   plan against the immutable Startup `DcTopology` before emitting an action:
   every planned position and station must match a System-Time-capable slave,
   the reference must exist in both plan and topology, and entries must be
   unique and capacity-safe.
9. The controller shall use a bounded non-blocking sequence with exact WKC 1:
   disable SYNC on every planned slave, write both cycle registers for every
   slave, read the selected reference clock once, write each shifted start
   time, then write each exact 16-bit `AssignActivate` word at `0x0980`.
10. One common unshifted epoch shall be strictly in the future, aligned to the
    checked least common multiple of all slave repeat periods, and far enough
    ahead to cover the bounded remaining start/activation requests. Per-slave
    start times shall equal that epoch plus the configured signed SYNC0 shift.
11. Accepted hardware writes shall never be described as rolled back. Public
    programmed evidence shall remain empty until every final activation write
    succeeds; faults may retain only bounded diagnostic progress and the first
    typed error. Explicit restart shall clear prior public evidence.
12. Action identity, generation, operation, address, payload/response length,
    WKC, request deadline, operation deadline, arithmetic, and monotonic plan
    invariants shall fail closed.
13. The new controller shall be an additive production service after DC clock
    initialization and before the legacy single-station DC service. It shall
    have its own Startup requirement bit and shall clear the lifecycle
    Configuration gate while incomplete, missing, retrying, or faulted.
14. Legacy products and callers that use no topology-wide plan or the existing
    `DcController` shall remain source-compatible and behavior-compatible.
15. Documentation and capability evidence shall state the exact software
    boundary and shall not claim external application-time discipline,
    runtime lock, hardware timing precision, HIL, ETG conformance, or
    functional-safety qualification.

## Acceptance Criteria

- [x] Timing tests cover direct cycle, positive/negative factors, SYNC1
      derivation, signed shifts, inexact division, invalid activation, zero,
      underflow, and overflow.
- [x] Cfggen tests prove resolved values in every generated artifact,
      deterministic hashes, strict rejection, and updated golden files.
- [x] Product-config tests prove ordered plan construction, non-DC omission,
      reference selection, duplicate/mismatch rejection, and pre-Startup
      validation.
- [x] Core tests prove the exact topology-wide register sequence, one reference
      read, LCM-aligned common epoch, per-slave shifts, future lead-time guard,
      WKC/shape/ownership/timeouts, atomic publication, and restart clearing.
- [x] Production-service and lifecycle tests prove fixed ordering, missing
      controller rejection, PREOP barrier behavior, and Configuration gating.
- [x] A public Linux simulated-port test carries the generated dual-axis plan
      through the shared request/RX path and observes complete two-drive
      evidence before Startup release.
- [x] The dual-axis example emits a two-entry 1 ms plan with exact
      `AssignActivate=0x0300`; the IO slave emits none.
- [x] README, PRD/requirements/product docs, backend spec, and capability
      manifest describe the implemented boundary without stronger claims.
- [x] Focused tests, no-std checks, `make cfggen-runtime-example`, repository
      `make ci`, `make bpf`, and exact pushed-SHA GitHub Actions all pass.

## Acceptance Evidence

- `cargo test -p esop-ethercat-core --lib`: 243 passed.
- Cfggen tests: 7 unit and 14 generation tests passed.
- Product configuration tests: 14 unit and 9 generated-product tests passed.
- Linux scheduled-domain integration tests: 17 passed, including the public
  topology-wide simulated service path.
- Lifecycle guard unit tests: 29 passed.
- `make cfggen-runtime-example`: generated hash
  `e1cabe9b4f85a44587e5a75ed5ecfc73de496bee557c75545ae58ac296b9c768`
  and exact checked-in Rust golden comparison passed.
- `make ci`: passed formatting, diff, workspace check/test/Clippy/release,
  AArch64 `no_std`, BPF syntax, manifests/schemas, generated build report,
  performance report, eBPF qualification reports, R2 report, and Zenoh check.
- `make bpf`: passed the CO-RE object build target.
- Trellis task validation and capability-manifest validation passed.
- Exact pushed-SHA GitHub Actions passed for
  `6e900742414d91e198fc749626b5bc42c266df0d`: quality run
  `36284581048` completed successfully, including all eBPF/BPF jobs, the Rust
  quality gate, Zenoh router integration tests, and build-evidence upload.

## Out Of Scope

- Authenticating or disciplining the caller's application time with PTP, TAI,
  GNSS, or another external clock source.
- Periodic all-slave drift compensation, `0x092c` sync-window monitoring,
  lock/recovery policy, or changing the existing cyclic FRMW monitor.
- FMMU automatic discovery, redundant-ring correction, hot connect, or online
  plan recalculation after Startup.
- Physical response provenance, device interoperability, target WCET,
  long-duration HIL, ETG conformance, or functional-safety qualification.

## Technical Notes

- Public IgH/SOEM behavior is used only as protocol-behavior evidence. ESOP's
  types, state machine, tests, comments, and layout remain independently
  authored.
- A common epoch is distinct from a common shifted first edge: each slave keeps
  its verified SYNC0 shift while all repeat schedules share one aligned epoch.
- The existing single-station `DcController` remains available for tests,
  maintenance, and explicit legacy use; it is not silently repurposed.
