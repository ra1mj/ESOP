# Implementation plan: generated product mailbox configuration

## 1. Core contracts

- [x] Add typed `MailboxConfigError` and shared range/capacity validation.
- [x] Reuse validation from `MailboxController::start` while preserving payload checks.
- [x] Add exact SII standard mailbox constants, protocol helpers, descriptor parser,
      `SiiBlockReader` metadata accessors, and focused unit tests.
- [x] Export new core types and constants through `esop-ethercat-core::lib`.

## 2. ESI and generator

- [x] Extend ESI SyncManager parsing with address, size, and control metadata.
- [x] Detect device-level CoE support and validate exactly one mailbox pair.
- [x] Add positive and fail-closed parser/generator tests.
- [x] Propagate mailbox metadata into generated slave JSON/inventory, semantic hashes,
      C header, and Rust module.
- [x] Update the simulator ESI and regenerate/check all six artifacts.

## 3. Product runtime

- [x] Add generated mailbox configuration to `ProductSlaveConfig`.
- [x] Add the no-binding batch builder and refactor explicit bindings onto shared
      validation.
- [x] Update unit/integration literals and prove generated and explicit batch paths.
- [x] Run focused `esop-ethercat-core`, `esop-cfggen`, and `esop-product-config` tests.

## 4. Documentation and verification

- [x] Update `.trellis/spec/backend/product-configuration.md` and product/protocol PRD
      status text without overstating live discovery.
- [x] Add/update the capability manifest and validate it.
- [x] Run `cargo fmt --all -- --check`, focused Clippy/tests, generated artifact checks,
      `make ci`, `make bpf`, `make test-hil`, and `make test-zenoh`.
- [x] Run `git diff --check` and prepare the verified implementation commit.
- [ ] Archive the Trellis task, record the journal, push `main`, and verify the matching
      GitHub Actions run.

## Risk and rollback points

- ESI direction semantics are easy to invert. Tests must assert master-send maps from
  `MBoxOut` and master-receive maps from `MBoxIn` in both ESI and SII paths.
- Generated hashes and checked-in golden artifacts will change. Regenerate them only
  after focused parser/runtime tests pass.
- Do not couple this task to Startup phases or claim physical SII observation; that is a
  separately reviewable activation increment.
