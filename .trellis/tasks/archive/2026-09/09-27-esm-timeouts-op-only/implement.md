# Implementation plan

## Phase 1: Core protocol primitives

- [x] Add versioned AL transition timeout profile and transition-selection tests.
- [x] Add MappingTable OpOnly output metadata and validation.
- [x] Add allocation-free OpOnly SyncManager controller with exact completion checks.
- [x] Add focused unit tests for writes, readbacks, masks, deadlines, and failures.

## Phase 2: Startup integration

- [x] Add `StartupSlaveProfile` and strict profile-aware startup entry point.
- [x] Preserve legacy startup and explicit uniform override behavior.
- [x] Integrate disable-before-non-OP, enable-after-OP, and already-OP verification.
- [x] Share one deadline across OpOnly and AL work for each transition step.
- [x] Prove failures cannot release startup readiness or overwrite first AL fault data.

## Phase 3: ESI/SII and mapping configuration

- [x] Parse ESI state-machine timeouts and ordered SyncManager OpOnly metadata.
- [x] Interpret SII activation enable/OpOnly bits separately.
- [x] Validate OpOnly direction and propagate metadata into mapping candidates.
- [x] Program and verify OpOnly outputs disabled during PREOP mapping configuration.
- [x] Add parser and configuration-controller regression tests.

## Phase 4: Generated product configuration

- [x] Add timeout profile and OpOnly metadata to generated/inventory data and hashes.
- [x] Add fields and validation to `ProductSlaveConfig`/`StaticProductConfig`.
- [x] Add the product-configured startup bridge.
- [x] Update the simulated dual-axis ESI and checked-in expected generated artifacts.
- [x] Add integration tests for exact generated runtime propagation.

## Phase 5: Evidence and closeout

- [x] Update requirements, capability matrix, and remaining-gap documents.
- [x] Run focused tests after each layer and repair regressions.
- [x] Run full Trellis quality checks and all project CI-equivalent gates.
- [ ] Commit implementation, archive the Trellis task, update the journal, push `main`,
      and verify the resulting GitHub Actions run.
