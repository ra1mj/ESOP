# Implementation plan: Startup SII mailbox verification

## 1. Shared mailbox semantics

- [x] Add a physical-layout equality helper to `MailboxConfig` and focused policy-ignore
      tests.
- [x] Keep validation centralized in the existing mailbox range contract.

## 2. Startup integration

- [x] Extend `StartupSlaveProfile`, phase/action/progress/error contracts and reset
      state.
- [x] Add exact-range `SiiBlockReader` ownership, pending/enqueue/accept/timeout routing
      and per-position verified evidence.
- [x] Insert mailbox verification after identity and before AL only when configured.
- [x] Add exact success, mismatch, CoE, timeout/request ownership, multi-slave and
      compatibility tests.

## 3. Product propagation

- [x] Attach generated mailbox values in `startup_profiles` with pre-mutation
      validation and typed product errors.
- [x] Update unit and checked-in generated-product integration assertions.

## 4. Documentation and verification

- [x] Update Trellis product contract, PRD/requirements/product docs and capability
      manifest with the new software evidence and remaining physical boundary.
- [x] Run focused core/product tests and generated artifact checks.
- [x] Run `cargo fmt --all -- --check`, `git diff --check`, capability validation,
      `make ci`, `make bpf`, `make test-hil`, and `make test-zenoh`.
- [x] Commit, push `main`, verify the matching GitHub Actions run, archive the task and
      record the Trellis journal.

## Risk and rollback points

- Identity and mailbox readers share the same ESC EEPROM registers. Distinct Startup
  action variants and strict pending equality must prevent cross-consumption.
- Generated startup opts into five additional EEPROM words per slave. Timeouts must use
  existing bounded budgets and never block or allocate.
- Do not compare runtime policy fields that SII cannot represent, and do not claim an
  EEPROM response is physically authentic merely because software parsed it.
