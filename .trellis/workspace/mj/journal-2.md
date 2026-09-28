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
