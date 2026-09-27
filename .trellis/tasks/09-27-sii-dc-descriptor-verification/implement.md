# Implementation plan: SII DC descriptor verification

## Core parser and contracts

- [x] Add borrowed Strings and fixed 24-byte DC category parsers with typed
      errors and public exports.
- [x] Add `SiiDcMode` / `SiiDcModeExpectation` plus exact mode lookup over a
      complete SII image.
- [x] Cover signed fields, multiple modes, malformed shape/index/reserved data,
      unknown names and duplicates.

## Generator and product contract

- [x] Extend strict ESI parsing for ordered `Dc/OpMode` records and all
      SII-representable fields/factors.
- [x] Add optional product `dc.op_mode`, validate required/non-required policy,
      resolve exactly one ESI mode and include it in semantic/config hashes.
- [x] Emit selected descriptor through JSON, inventory, C and Rust artifacts.
- [x] Extend `ProductSlaveConfig` and product startup validation/propagation.

## Startup integration

- [x] Add expected/verified DC descriptor slots and restart clearing.
- [x] Reuse the existing configuration stream scratch; stage SII signature and
      DC descriptor checks before committing either evidence.
- [x] Add success, mismatch, malformed, no-second-pass, pre-AL and compatibility
      tests.

## Product fixtures and documentation

- [x] Add a named DC mode to the simulator drive ESI and select it for both
      generated drive instances.
- [x] Regenerate checked-in product artifacts and update generator goldens.
- [x] Update PRD traceability, README, EtherCAT/product docs, backend spec and
      capability manifest while retaining physical/HIL limits.

## Validation

- [x] Run focused core, cfggen, product-config and generated-product tests.
- [x] Run `cargo fmt --all -- --check` and relevant no-std checks.
- [x] Run `make ci` and `make bpf`.
- [x] Review the complete diff for accidental source copying, unbounded work,
      generated timestamp/path leakage and claim inflation.
- [x] Commit, push `main`, verify GitHub Actions on the exact SHA, record task
      acceptance, archive the task and push the final metadata commit.

## Risk and rollback points

- The ESI parser has overlapping `Name`/`Desc` element names. Keep the DC
  builder state explicit and test device/PDO/entry names for regressions.
- Startup currently commits SII signature evidence immediately after finalize.
  Stage both observations first so a later DC mismatch cannot leave partial
  verification evidence.
- Generated C/Rust record changes touch all checked-in products and tests. Use
  the generator as the source of truth; do not hand-edit generated files.
- Do not combine this task with topology-wide SYNC register programming. That
  next task consumes the verified descriptor contract after this one passes.
