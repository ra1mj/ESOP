# Runtime diagnostics CLI and eBPF status

## Goal

Implement the first stable, read-only `esop` operator command surface. It shall
provide ROS2-style status inspection and incremental watch commands over a
versioned runtime snapshot that combines EtherCAT Domain/WKC, DC, lifecycle,
and eBPF observation state without executing work in the real-time cycle.

## Confirmed Facts

- ProcBuf v7 already carries link/AL/DC status, DC offset, deadline misses,
  per-Domain expected/actual WKC, mismatch streak and age, lifecycle state, and
  a bounded runtime-observation summary.
- `ProcBufProjector` is the sole host-side state projection owner and currently
  drops those detailed operational fields when building `RobotState`.
- `RuntimeIncident` already carries correlated eBPF evidence and is available
  in `QueryReply` through the Zenoh query route.
- Zenoh typed queries currently require an exact boot ID, which prevents a new
  read-only CLI process from discovering the current boot without an external
  configuration source.
- No workspace crate currently owns a general `esop` executable or stable
  diagnostic command registry.

## Requirements

### R1. Additive operational status contract

Extend `RobotState` additively with one typed `OperationalStatus` containing:

- link state, observed AL state, fault bitmap, command age and deadline misses;
- DC lock and signed offset;
- one ordered `DomainStatus` per ProcBuf Domain with exact expected/actual WKC,
  validity, completeness, mismatch streak, last-valid cycle and input age;
- bounded eBPF agent epoch, observation window, incident/loss counters and
  observation health.

Existing field numbers and the frozen v1 baseline must remain compatible.

### R2. Single projection owner

`esop-ipc::payloads::ProcBufProjector` shall populate the operational status
from the same accepted `StateSnapshot` used for joints, IO, lifecycle and
quality. CLI rendering and Zenoh code shall not reinterpret ProcBuf bytes or
duplicate lifecycle/WKC validation.

### R3. Read-only boot discovery

A typed query with `boot_id == 0` shall mean “bind this read-only query to the
current provider boot”. The server-side decoder shall normalize it to the
provider boot before validation and reply construction. Nonzero mismatches
remain rejected. This relaxation applies only to status queries and grants no
command or motion authority.

### R4. Stable command registry

The initial `esop` executable shall support:

- `esop status`: compact master, quality, lifecycle, DC and host observation;
- `esop domain list`: ordered per-Domain WKC and freshness table;
- `esop dc`: DC lock and offset status;
- `esop lifecycle`: lifecycle gates, permit and fault state;
- `esop incident list`: bounded correlated eBPF incidents;
- `esop doctor`: nonzero exit when required runtime facts are absent or bad;
- `esop watch`: repeat the selected read-only view using an incremental state
  cursor and a bounded polling interval.

The first release uses the existing Zenoh typed query route. Parser and
renderer logic shall remain transport-independent so local IPC can be added
without changing command names.

### R5. Bounded live behavior

Queries shall retain the existing maximum record count and payload limit.
`watch` shall enforce a positive bounded interval, process one reply at a time,
and never busy-spin. Remote errors, missing replies, invalid schema, robot/boot
mismatch, and malformed operational status shall produce distinct nonzero
process exits.

### R6. Safety and observability boundary

The CLI is read-only. It shall not load BPF, write ProcBuf commands, submit SDO
or register requests, alter AL state, acknowledge faults, or change lifecycle
permits. eBPF health degradation and lost events must be visible rather than
silently interpreted as a healthy host.

## Acceptance Criteria

- [x] Protobuf compatibility tests prove the operational fields are additive
  and old readers still decode new `RobotState` messages.
- [x] ProcBuf projection tests prove exact Domain, DC, AL/link, deadline and
  runtime-observation values survive into Protobuf without local CLI casting.
- [x] Query tests prove zero boot discovery is normalized, while stale nonzero
  boot IDs remain rejected.
- [x] Command parser tests cover every command, help, missing arguments,
  unknown commands, invalid interval and bounded limit.
- [x] Renderer tests cover healthy and degraded DC/WKC/lifecycle/eBPF states.
- [x] A Zenoh integration test queries and renders a live projected state plus
  one correlated runtime incident.
- [x] `make ci`, `git diff --check`, and focused command tests pass.
- [x] Capability and operator documentation state the read-only and
  non-safety qualification boundaries.

## Out of Scope

- EtherCAT configuration or maintenance write commands.
- Per-slave ESC register dumps not present in the current host snapshot.
- Loading or attaching eBPF programs from the CLI.
- Local IPC query serving, shell completion, TUI, JSON/YAML output, ROS 2 node
  discovery, or functional-safety claims.
