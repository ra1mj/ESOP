# Implementation plan

## 1. Core protocol types and transfer

- Add typed CANopen data types, access flags, SDO Information policy, opcodes,
  descriptions, transfer phases/progress, and errors.
- Encode OD-list, object-description, and entry-description requests.
- Decode fixed metadata, aborts, and bounded fragments transactionally.
- Add exact-byte, capacity, malformed, abort, and fragment-sequence tests.

## 2. Core generated-plan verifier

- Add immutable expectation and required-access types.
- Preflight ordering, uniqueness, widths, policy, and plan capacity.
- Sequence object and entry description requests and validate all metadata.
- Keep successful publication empty until the whole expectation slice passes.
- Add positive multi-object and focused mismatch tests.

## 3. ESI and product schema

- Parse direct `CoE@SdoInfo` for empty and paired elements.
- Preserve exact CANopen data type on `EsiEntry` while retaining signedness.
- Add strict `coe.sdo_information` with a fail-closed default.
- Reject unsupported enablement and update simulator ESI/product fixtures.

## 4. Expectation derivation and generated artifacts

- Deduplicate selected RxPDO/TxPDO entries in deterministic object order.
- Merge required access when an entry appears in both directions.
- Propagate support, enablement, and plans through normalized product,
  inventory, robot input, C, Rust, semantic hash, and config hash.
- Extend strict robot report generation/validation and regenerate goldens.

## 5. Product runtime and integration

- Add generated slave fields and expectation slices to product configuration.
- Validate feature capability and plan ownership/shape before activation.
- Expose typed per-slave SDO Information policy and plan lookup.
- Add a mailbox/master integration test for a complete verification sequence.

## 6. Documentation and quality gates

- Update FR-017 / COE-004, capability evidence, product configuration docs,
  and the Trellis product contract with explicit qualification limits.
- Run focused Rust/Python tests and generated artifact checks.
- Run `cargo fmt --all -- --check`, `make no-std`, `make lint`, and `make ci`.
- Review diffs, archive the task, record the journal, commit, push to
  `ra1mj/ESOP`, and verify the exact final GitHub Actions SHA.

## Risk And Rollback Points

- Protocol decoding must reject malformed fragments before mutating published
  metadata; core tests are the first rollback gate.
- Generated schema changes touch committed goldens and robot validation;
  regenerate only with repository tools and inspect semantic/config hash diffs.
- Product runtime validation must run before activation publishes state; retain
  the prior generated artifacts until focused activation tests pass.
- The optional manifest field defaults false, so disabling it is the immediate
  product rollback without cyclic or ABI migration.
