# Existing Configuration Reuse Research

## Retained startup evidence

`StartupController` retains verified slave records and profiles, mailbox/SII
evidence, SyncManager/FMMU register banks, selected DC reference, and full DC
topology. `start_mapping_for_position` already resolves per-position register
evidence. `SlaveRecord.configured` is the existing readiness bit, but requires
narrow target-only invalidation and commit APIs.

## Reusable controllers

- `PdoConfigController` already configures one station from one bounded plan.
- `MappingConfigController` already clears, writes, and verifies one station's
  discovered SM/FMMU registers.
- `WatchdogController` can run a single-entry plan.
- `OpOnlySyncManagerController` can disable/read back OpOnly channels before a
  lower AL-state request.
- `StateRequestController` can drive PREOP directly with copied verified
  context after OpOnly channels are disabled.
- `DcClockController` and `DcSyncController` currently build full-slave plans;
  they need additive target-only start paths that still validate the full
  retained topology/reference context.

## Product and scheduler boundaries

`StaticProductConfig` already builds a per-position PDO startup plan and full
watchdog/DC plans. The runtime owns the frozen `MappingTable`, so the targeted
builder must accept it rather than reconstruct process-image ownership.

The production scheduler runs Domain/DC first, then one service request, and
retains an accepted request across cycles. Reconfiguration should be a named
service that delegates to the current child controller instead of creating a
parallel transport or queue.

## Decisions

- Stop at verified PREOP; no automatic SAFEOP/OP return.
- Invalidate only the target after complete admission and before any wire
  action.
- Leave the target unconfigured after any terminal fault; no automatic retry.
- Permit reading a non-target DC reference but never writing a non-target.
- Keep the public operation fixed-capacity and allocation-free.
