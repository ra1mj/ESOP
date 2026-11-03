# Journal - mj (Part 2)

> Continuation from `journal-1.md` (archived at ~2000 lines)
> Started: 2026-09-27

---



## Session 59: EtherCAT asynchronous register requests

**Date**: 2026-09-27
**Task**: EtherCAT asynchronous register requests
**Branch**: `main`

### Summary

Implemented a no_std fixed-capacity asynchronous fixed-station ESC register request queue, integrated it as the lowest-priority production control service with request-local failures and no in-flight retransmission, added cross-cycle Linux simulation coverage and capability/spec documentation, and passed make test-hil plus make ci.

### Main Changes

- Detailed change bullets were not supplied; see the summary above.

### Git Commits

| Hash | Message |
|------|---------|
| `dbb4422` | (see git log) |

### Testing

- Validation was not recorded for this session.

### Status

[OK] **Completed**

### Next Steps

- None - task complete


## Session 60: EtherCAT runtime state requests

**Date**: 2026-09-27
**Task**: EtherCAT runtime state requests
**Branch**: `main`

### Summary

Implemented bounded explicit request_state over the AL FSM, Startup retained evidence, production scheduling/lifecycle gating, cross-cycle Linux simulation, and REC-001 partial capability/docs; passed make test-hil and make ci.

### Main Changes

- Detailed change bullets were not supplied; see the summary above.

### Git Commits

| Hash | Message |
|------|---------|
| `637bce4` | (see git log) |

### Testing

- Validation was not recorded for this session.

### Status

[OK] **Completed**

### Next Steps

- None - task complete


## Session 61: EtherCAT explicit rescan

**Date**: 2026-09-28
**Task**: EtherCAT explicit rescan
**Branch**: `main`

### Summary

Implemented bounded PREOP-only EtherCAT rescan with synchronous evidence invalidation, Startup reuse, production scheduling, lifecycle gating, delayed-response simulation, executable specs, and complete local CI.

### Main Changes

- Detailed change bullets were not supplied; see the summary above.

### Git Commits

| Hash | Message |
|------|---------|
| `bd8fd8f` | (see git log) |

### Testing

- Validation was not recorded for this session.

### Status

[OK] **Completed**

### Next Steps

- None - task complete


## Session 62: EtherCAT single-slave reconfiguration

**Date**: 2026-09-28
**Task**: EtherCAT single-slave reconfiguration
**Branch**: `main`

### Summary

Implemented bounded target-isolated EtherCAT reconfiguration through direct PREOP, PDO/watchdog/SM-FMMU/target-DC sequencing, production scheduling, lifecycle gating, Linux coexistence tests, executable specs, and requirement/capability evidence.

### Main Changes

- Detailed change bullets were not supplied; see the summary above.

### Git Commits

| Hash | Message |
|------|---------|
| `13d1b01` | (see git log) |

### Testing

- Validation was not recorded for this session.

### Status

[OK] **Completed**

### Next Steps

- None - task complete


## Session 63: EtherCAT explicit recovery integration and closeout

**Date**: 2026-09-28
**Task**: EtherCAT explicit recovery integration and closeout
**Branch**: `main`

### Summary

Added unified allocation-free recovery status/result/fault projections and fixed-capacity diagnostics, qualified mixed recovery scheduling under cyclic load, synchronized REC-001 documentation and capability boundaries, passed local and exact GitHub quality gates, and archived the integration child plus parent recovery plan.

### Main Changes

- Detailed change bullets were not supplied; see the summary above.

### Git Commits

| Hash | Message |
|------|---------|
| `bcb9c9e` | (see git log) |

### Testing

- Validation was not recorded for this session.

### Status

[OK] **Completed**

### Next Steps

- None - task complete


## Session 64: Runtime diagnostics CLI and eBPF status

**Date**: 2026-09-28
**Task**: Runtime diagnostics CLI and eBPF status
**Branch**: `main`

### Summary

Added read-only ESOP status/domain/DC/lifecycle/incident/doctor/watch commands, additive operational telemetry, zero-boot typed Zenoh discovery, live query/render coverage, and retained the next SOEM/DC parity task.

### Main Changes

- Detailed change bullets were not supplied; see the summary above.

### Git Commits

| Hash | Message |
|------|---------|
| `4159e0a` | (see git log) |

### Testing

- Validation was not recorded for this session.

### Status

[OK] **Completed**

### Next Steps

- None - task complete


## Session 65: Linux R0-R2 software master closure

**Date**: 2026-10-26
**Task**: Linux R0-R2 software master closure
**Branch**: `feat/linux-r0-r2-master`

### Summary

Completed Linux software master, independent simulation, mailbox reliability, motion lifecycle, replay, Q1/Q2/Q4 and virtual-hour soak; all Linux CI jobs passed; hardware qualification remains deferred.

### Main Changes

## Delivered

Implemented the approved Linux software/simulation R0-R2 P0 master closure:
correct EtherCAT M/address/RX/scan semantics; reserved control wire leases;
independent fixed-capacity no_std ESC/bus; full SM/MRL/CoE and eight 1 KiB runtime
SDO requests; age-qualified AL/DL monitoring; explicit maintenance Fault Reset;
production-owner CSP/CSV/CST sim/replay/raw example; actual Q1/Q2/Q4 workloads;
virtual I2C lifecycle isolation; exact classic-PCAP replay; artifact-bound reports
and fail-closed qualified release gate. Updated generated products, capability
manifest, original P0 evidence map, and project specifications.

## Validation

- Portable workspace: 771 tests passed; Clippy/check/fmt passed. Linux-only port
  and Aya packages excluded locally; three zenohd tests ignored and one macOS
  socket-size case filtered. Actual unfiltered Linux CI is recorded below.
- All 100 Python tests passed. Six product sets / 36 generated artifacts match;
  six C11 application consumers compile and run under strict warnings.
- Core/product/simulator and guard feature sets cross-check as no_std.
- Thirteen real production-master scenarios, two exact replays, changed-TX and
  incomplete-capture rejection, and overflow-invariant full digests/counters pass.
- Q4 release soak: 3,600,000 cycles at 1 ms virtual period; 16 axes/17 slaves;
  35,520 completed SDO requests, zero failures, eight explicitly outstanding at
  shutdown; zero cyclic/DC/mailbox errors and zero activated heap operations.
  Bounded capture retains 100,000 of 22,084,638 records; full-stream hashes,
  cycle samples and error counts remain complete. Evidence is versioned at
  qualification/software_master_soak.json, bound to source commit 6efc82c.
- Require-qualified release gate correctly rejects the development baseline.

## Boundaries

Software evidence remains hardware_qualified=false. Real interoperability,
physical DC precision, target jitter/WCET, wall-time HIL and STM32/HPMicro ports
remain deferred. Original untracked paseo.json was left untouched.

## GitHub Linux CI

All 12 jobs passed on pushed code head 585de6191f74442e8631535ec9040a9efbd9f94c:
https://github.com/ra1mj/ESOP/actions/runs/40278257137

Linux workspace: 852 passed, zero failed; the three initially ignored
router-dependent tests subsequently all passed in dedicated live Zenoh tests.
The macOS-filtered oversized IPC case passed unfiltered on Linux. Additional
product tests 38/38 and Python validators 100/100 passed. Full make ci includes
Clippy/fmt/check/release/no_std, all six product comparisons, 13 software cases,
two exact replays and capture rejection tests. BPF build and all ten privileged
eBPF runtime qualifications passed with validated reports and artifact uploads.
Build evidence artifact 12631082275 contains 153 files.

All work commits were pushed to feat/linux-r0-r2-master. Hardware qualification
was intentionally not granted; all applicable software acceptance criteria pass.


### Git Commits

| Hash | Message |
|------|---------|
| `6efc82c` | (see git log) |
| `235a5b5` | (see git log) |
| `585de61` | (see git log) |

### Testing

- Validation was not recorded for this session.

### Status

[OK] **Completed**

### Next Steps

- None - task complete


## Session 66: Linux master hardening and main integration

**Date**: 2026-11-03
**Task**: Linux master hardening and main integration
**Branch**: `main`

### Summary

Hardened mailbox scheduling, monitor freshness and capture/evidence boundaries; final-code soak and all Linux CI jobs passed; fast-forwarded and pushed the completed work to main.

### Main Changes

## Delivered

Continued the approved Linux software master hardening and integrated the
feature history into main with a normal fast-forward, without force pushing.

- Idle mailbox expiration now releases ownership for both already-queued and
  later same-station SDO requests. The real scheduler/VirtualBus regression
  preserves the original deadline and queued Emergency evidence.
- AL/DL monitoring carries immutable observation/expiry bounds through the
  motion owner and is rechecked around auxiliary TX, motion TX and publication.
  Accepted Controlword and stop evidence remain truthful when time expires.
- Unresolved wire-index reuse suppresses ambiguous RTT for the observer session.
  Histograms record actual optional observations and expose coverage; legacy
  reports retain explicit unknown coverage.
- Failed, allocating or SDO-loaded baselines cannot define performance regression.
  Unsupported scenarios are rejected before port setup; aliases use one canonical
  name across capture, run and offline topology metadata. Added CLI help.

## Verification

Focused final review passed 545 core/sim/guard tests, 16 example tests, 8 virtual
I2C tests, 103 Python tests, Clippy/type checks, no_std checks and 55 portable
legacy Linux fixtures. The full software gate passed 13 cases, alias/invalid
input checks, two exact replays and capture rejection tests.

All 12 Linux CI jobs passed for final code deaf254:
https://github.com/ra1mj/ESOP/actions/runs/40283639418

The final-code Q4 soak passed 3,600,000 virtual 1 ms cycles and measured RTT in
all 3,600,000 cycles. It completed 35,520 SDO requests with zero request failures
and retained eight outstanding at shutdown. Activated allocations, frees and
reallocations were zero. Cyclic/DC/mailbox errors were zero. The bounded prefix
contains 100,000 of 22,084,638 wire records; all omitted records are counted and
full-stream/state/event digests are retained. Wire/state/event digests match the
previous soak. Offline report and versioned evidence were regenerated and
validated against deaf254, then committed in3751559.

## Boundaries

Hardware qualification remains false. Physical interoperability, DC precision,
target timing and MCU ports remain deferred. User-owned paseo.json was preserved.

## Main integration

On 2026-11-02 the refreshed origin/main at c7d43e5 was fast-forwarded to
37515598f2abb85761443fad11d4ef17264d37a6 and pushed without force. All 12 main CI
jobs passed, including the Rust quality gate and Zenoh integration:
https://github.com/ra1mj/ESOP/actions/runs/41467142313

All applicable software acceptance criteria and requested main integration are
complete. Archive and journal commits contain only task/session bookkeeping.


### Git Commits

| Hash | Message |
|------|---------|
| `acae720` | (see git log) |
| `2beca02` | (see git log) |
| `deaf254` | (see git log) |
| `3751559` | (see git log) |

### Testing

- Validation was not recorded for this session.

### Status

[OK] **Completed**

### Next Steps

- None - task complete


## Session 67: README project homepage

**Date**: 2026-11-03
**Task**: README project homepage
**Branch**: `main`

### Summary

Rewrote the Chinese README around current capabilities, runnable examples, architecture, validation and documentation links; link/command checks, simulation/replay and all CI jobs passed; pushed main.

### Main Changes

## Delivered

Rewrote README.md as a concise Chinese project homepage: current Linux software
master status, main-branch CI badge, capability table, runnable help/simulation/
replay examples, workload overview, logical architecture diagram, grouped crate
navigation, quality/evidence commands, documentation links and qualification
boundaries. Historical register and eBPF fixture detail now points to existing
specialized documentation. CLI remains explicitly read-only; observation health
feeds the lifecycle input rather than claiming ownership of ProcBuf State.

## Validation

- Validated 44 links/anchors and seven Make targets; checked balanced code fences.
- Checked CLI --help and matched simulation/replay scenario, mode, cycles and
  capture paths.
- Executed 10,000-cycle release simulation and exact replay in isolated
  build/readme-validation directories. Scenario/state/event summaries matched;
  both passed with zero activated heap operations and hardware_qualified=false.
- git diff --check passed. No runtime code or public interface changed, and no
  new tests were added for the documentation-only edit.
- Reviewed spec sync; no new implementation contract required a spec update.

Original user-owned paseo.json remains untracked and untouched.

## Delivery

Work commit968008d was pushed to main; all12 GitHub CI jobs passed:
https://github.com/ra1mj/ESOP/actions/runs/41650938043

GitHub README blob6e3f9eff888f123f05bfbbd3a3c98a9d7b8defa2 matches the local file.


### Git Commits

| Hash | Message |
|------|---------|
| `968008d` | (see git log) |

### Testing

- Validation was not recorded for this session.

### Status

[OK] **Completed**

### Next Steps

- None - task complete
