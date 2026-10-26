# Linux R0-R2 software master completion

## Goal

Implement the user-approved R0-R2/P0 Linux software and simulation master plan
against `docs/esop-software-prd.md`. Deliver a runnable, replayable,
fault-injectable master without claiming physical interoperability, DC accuracy
or target WCET.

## Requirements

- R1: Correct M flags, physical address words and command-aware RX validation.
  Freeze a bounded initial BRD transit count; no response is a scan fault.
  Use independent golden bytes for all commands and multi-datagram frames.
- R2: Fixed-capacity cross-platform virtual ESC/bus driven only by frames,
  virtual time and explicit faults, independently calculating register access
  and WKC. Cover SII/AL/SM/FMMU/PDO/DC/mailbox/drive/IO. Exact classic PCAP
  replay requires a product/time/scenario/direction sidecar and preloading.
- R3: Complete mailbox ownership, MRL repeat/ACK, normal/segmented CoE, typed
  errors, eight bounded SDO requests, shared per-slave transport authority,
  fairness, request-local runtime faults and unsolicited Emergency.
- R4: Add bounded age-qualified AL/DL monitoring, slave snapshots and contextual
  events projected into lifecycle. Exercise a virtual I2C device lifecycle and
  Pending/NACK/timeout isolation.
- R5: Caller-driven r0_r2 sim/replay/raw example reusing production scheduling
  and cycle ownership, full startup/readback/OP/PDO/DC/CSP/CSV/CST/stop/recovery,
  captures, measurement and report import.
- R6: Preserve Domain/DC priority, exact ownership/WKC/generation/deadlines,
  activation-frozen allocation-free runtime, complete-only publication,
  explicit recovery and new permits. Separate scenario success from hardware
  qualification; require qualification at the release gate.

## Acceptance Criteria

- [x] Golden/malformed frame tests cover all commands, M/address semantics,
  bounded counting, AP/BR position changes and no partial commit.
- [x] 1/8/32 virtual slaves scan; dual-axis+IO reaches OP only through the wire.
- [x] Mailbox consumed-read loss, repeat/ACK faults, zero/nonsequential/wrapping
  counters, normal/segmented objects, bounds, idle Emergency, queue overflow,
  eight requests and fault isolation pass with unchanged cyclic priority.
- [x] Commands/modes and WKC/DC/AL/link/drive/expiry/supervisor faults exercise
  stops and explicit recovery/new permit; fault disappearance never rearms.
- [x] Deterministic inputs reproduce wire/state/event evidence; exact capture
  replay matches and modified TX/late/corrupt RX fail precisely.
- [x] Q1/Q2/Q4 software scenarios, eight-request Q4, allocator checks and a
  one-hour soak entry exist. Sim/replay cannot qualify HIL or WCET.
- [ ] Focused tests, Linux integration and available full quality gates pass;
  original software-applicable P0 requirements have evidence mappings.

## Scope and Defaults

User approved implementation and deferred MCU ports and physical testing.
FoE/SoE/EoE/redundancy/new ROS2 remain later work. Default: dual-axis+IO,
1 ms virtual time. Existing public RX APIs default to exact validation. Keep
old SDO default capacity; the new scenario uses eight 1 KiB objects with the
existing bounded physical mailbox limit. Hardware P0 requirements stay open.
