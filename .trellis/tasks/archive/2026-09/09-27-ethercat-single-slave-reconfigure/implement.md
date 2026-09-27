# Implementation Plan: EtherCAT Single-Slave Reconfiguration

## 1. Core Operation Model

- [x] Add fixed-capacity reconfiguration plan, handle, phase, progress, result,
      submission error, and runtime error types.
- [x] Implement one-operation coordinator with transactional start, absolute
      deadline enforcement, stable/stale handle behavior, first-fault
      retention, and no retry.
- [x] Compose OpOnly, state-request, PDO, watchdog, mapping, and DC child
      controllers without duplicating protocol logic.

## 2. Targeted Authorities

- [x] Add narrow target configuration invalidation/commit APIs owned by
      Startup/SlaveTable.
- [x] Add target-only DC clock and DC Sync start paths that validate full
      retained topology/reference evidence but write only the target.
- [x] Add product-configuration helpers that build/filter one target's PDO,
      watchdog, and DC evidence while accepting the caller-frozen mapping.

## 3. Startup and Lifecycle Integration

- [x] Add `StartupController::start_reconfigure_slave` admission and retained
      evidence reconciliation.
- [x] Invalidate only the target before the first available action; commit only
      that target after verified PREOP success.
- [x] Make active/faulted reconfiguration close lifecycle topology readiness
      without publishing OP readiness on success.

## 4. Production Scheduling

- [x] Add the named reconfiguration production service after Rescan and before
      ordinary configuration/state/mailbox/register work.
- [x] Route PDO phases through mailbox transport and other phases through the
      control transport while retaining one accepted request across cycles.
- [x] Preserve Domain/DC-first ordering and one bounded control slot.

## 5. Focused Verification

- [x] Cover transactional rejection, target-only invalidation, stable/stale
      handles, phase ordering, absolute deadline, and first-fault retention.
- [x] Cover OpOnly disable-before-PREOP and prove no SAFEOP/OP request.
- [x] Cover target-only PDO/watchdog/mapping/DC writes and non-target DC
      reference read behavior.
- [x] Cover scheduler priority, delayed completion without retransmission, and
      lifecycle fail-closed behavior.
- [x] Add Linux cyclic coexistence simulation with an unchanged unrelated
      slave and terminal target PREOP/configured evidence.

## 6. Documentation and Delivery

- [x] Update affected EtherCAT/product/lifecycle Trellis specs with executable
      contracts learned during implementation.
- [x] Update capability/requirement documentation only for the software
      boundary proven by this child; leave unified recovery qualification to
      the final parent child.
- [x] Archive the child, record journal evidence, commit, push, and wait for
      exact GitHub Actions success.

## Validation Commands

```bash
cargo test -p esop-ethercat-core reconfigure
cargo test -p esop-product-config reconfigure
cargo test -p esop-lifecycle-guard reconfigure
cargo test -p esop-linux-runtime reconfigure
cargo check -p esop-ethercat-core --no-default-features
make test-hil
make ci
```

## Risk and Rollback Points

- Keep target invalidation behind fully successful plan/controller admission;
  admission defects can otherwise revoke readiness without an executable path.
- Land targeted DC entry points with focused tests before coordinator use;
  accidental full-plan writes would violate the core isolation requirement.
- Preserve child-controller exact action identity through the production
  scheduler; resetting a phase while a response is pending risks duplicate
  transmission.
- Do not restore `configured` on any partial failure. Rollback is an explicit
  rescan/new reconfiguration, not speculative compensating traffic.
