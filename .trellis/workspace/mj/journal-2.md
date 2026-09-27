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
