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
