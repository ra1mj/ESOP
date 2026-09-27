# Design: FMMU-mapped mailbox status bit

## Boundary

The mapped path is an additional post-Mapping status source, not a replacement
for the direct MBoxIn SyncManager status byte used while configuring PDOs. The
same canonical physical bit is represented twice across lifecycle phases:

1. direct fixed-address polling during bootstrap/configuration;
2. FMMU-projected Domain input during steady-state mailbox service.

There is no automatic fallback from the second path to the first.

## Generated Layout

For each Domain, cfggen keeps the existing RxPDO-then-TxPDO layout, rounds the
PDO end to a byte, and appends one bit for each `MBoxState` slave ordered by
position. The final status tail is byte-rounded once. The existing aggregate
LRD starts at the Domain input boundary and expands through the tail.

Each binding freezes:

- slave position and Domain ID;
- Domain-relative bit offset and `max_age_cycles = period_ticks`;
- the first supported ordered SyncManager-status FMMU index;
- a canonical read FMMU descriptor:
  - logical byte/bit from Domain base plus the generated offset;
  - physical byte `0x0800 + MBoxIn * 8 + 5`;
  - physical bit 3 (`0x08`), one mapped bit, type 1, enabled.

The LRD expected WKC adds one per mapped status FMMU. Status bits are not PDO
entries and do not change PDO handles, typed axis maps, or slave-copy fields.

## Product Validation

`StaticProductConfig` reconstructs the status tail from the maximum PDO end in
each Domain and the ordered set of status-enabled slaves. It then checks every
generated binding, input datagram coverage/WKC, Domain process-image length,
and overlap/uniqueness. The direct bootstrap policy remains canonical and is
validated together with the mapped binding.

The activated product retains validated bindings and exposes them by slave
position. A binding exposes its exact `FmmuConfig`, allowing the caller's
per-slave mapping table to use the existing `MappingConfigController` and
Startup-verified register banks without a second writer.

## Runtime State Machine

`MailboxController` gains an explicit polling source selected at start:

- PollTime: current behavior when no status bit is configured;
- Direct: current fixed-address one-byte read;
- Mapped: no status control action is emitted.

After the send completes, mapped mode waits for an observation. The scheduler
asks `ScheduledDomainBank` to resolve the binding against the committed input.
The observation is usable only when:

- Domain ID and logical placement match;
- Domain quality is valid and complete;
- a valid input has been committed;
- `current_cycle - last_valid_cycle < max_age_cycles`.

An inactive observation schedules the next bounded check. An active
observation advances to the existing input-mailbox read. An unavailable or
stale observation reports a typed non-terminal progress state and performs no
control request, preserving recovery when the next valid Domain generation
arrives.

## Compatibility And Failure Policy

- `MailboxConfig::new`, `with_status_bit`, and `MailboxController::start`
  preserve existing behavior.
- Mapped mode requires a canonical status policy and a validated binding.
- Missing/tampered generated data fails product activation.
- Invalid/stale cyclic input suppresses reads; it does not mutate the last
  committed image, emit a direct read, or permanently fault the mailbox FSM.
- Mapping write/readback failure remains terminal in the Mapping controller and
  prevents lifecycle release independently of mailbox polling.

## Verification

- Core unit tests cover canonical FMMU encoding and mapped mailbox phases.
- cfggen/product tests cover deterministic packing, hashes, domain/WKC changes,
  tamper rejection, and PollTime compatibility.
- Mapping tests use the exact generated descriptor through write/readback.
- Linux scheduled production tests prove active/inactive/invalid/stale Domain
  observations and absence of direct status reads.
- Repository gates cover formatting, generated artifacts, no_std, clippy, and
  full CI.

## Rollback

The feature is additive and selected only when a generated mapped binding is
used to start the runtime mailbox. Reverting the binding fields and mapped
start path restores the existing direct policy without changing mailbox wire
format, SII evidence, or PDO object layout.
