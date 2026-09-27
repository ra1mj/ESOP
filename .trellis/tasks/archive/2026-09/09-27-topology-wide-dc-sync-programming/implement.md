# Implementation plan: topology-wide DC SYNC programming

## Phase 1: Shared timing and plan contracts

- [x] Add typed resolved timing, plan entry/plan, errors, checked factor
      conversion, SYNC1 derivation, activation validation, LCM helpers, and
      focused unit tests in `esop-ethercat-core`.
- [x] Export the additive contracts from the core crate without changing the
      existing `DcController` API.

## Phase 2: Product generation and runtime plan

- [x] Resolve selected ESI modes through the shared timing contract in
      cfggen; add resolved values to normalized JSON, inventory, C, and Rust.
- [x] Add the static product field, plan builder, exact rebuild validation,
      typed product errors, and tests for ordered/non-DC/reference behavior.
- [x] Regenerate the dual-axis golden artifacts and prove hash determinism and
      strict invalid-factor/activation failures.

## Phase 3: Topology-wide controller

- [x] Implement the fixed-capacity all-slave state machine, exact register
      actions, common epoch calculation, staged evidence, restart, and typed
      failure paths.
- [x] Add unit tests for multi-slave order, shared reference read, shifted
      starts, LCM alignment, lead-time guard, action ownership, WKC, lengths,
      arithmetic, deadlines, empty plan, partial hardware progress, and
      restart.

## Phase 4: Scheduler, Startup, lifecycle, and integration

- [x] Add the dedicated production service and Startup requirement bit after
      DC clock initialization and before legacy DC configuration.
- [x] Extend lifecycle projection and tests so every non-complete state clears
      Configuration readiness.
- [x] Add a Linux simulated-port integration test that drives the generated
      two-drive plan through normal control requests/RX and releases Startup
      only after full evidence publication.

## Phase 5: Documentation and verification

- [x] Update README, product/requirements/PRD/architecture docs, backend spec,
      and capability manifest with the exact implemented boundary.
- [x] Run focused crate tests and generated-example checks.
- [x] Run `cargo check --workspace --all-features`, no-std target checks,
      `make cfggen-runtime-example`, `make ci`, `make bpf`, and
      `git diff --check`.
- [x] Run Trellis validation/check, record acceptance evidence, commit the
      implementation, push `main`, and require GitHub Actions success for the
      exact final SHA before archive/journal commits.

## Risk And Rollback Points

- Timing formulas are protocol-significant; keep one shared resolver and do
  not duplicate arithmetic in cfggen and runtime.
- `AssignActivate` is an exact two-byte register contract; do not reduce it to
  the legacy local enum.
- Common-epoch arithmetic must fail on overflow instead of saturating.
- A later failure cannot undo prior ESC writes; only publication is atomic.
- Keep the new service additive so legacy scheduler call sites remain valid.
