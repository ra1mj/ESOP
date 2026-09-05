# Implementation Plan

1. Add the profile-to-core dependency and export a new `pdo` module.
2. Implement standard object constants, fixed map storage, field validation, and mode-specific requirements.
3. Implement typed TxPDO decoding and separate control/cyclic RxPDO encoding with explicit motion gates.
4. Add unit tests for all three modes, signed values, bit preservation, malformed maps/images, and atomic failure behavior.
5. Update the PRD traceability/docs and project quality spec with the new boundary.
6. Run `cargo fmt --all`, targeted profile/procbuf tests, `cargo clippy --workspace --all-targets --all-features -- -D warnings`, and `make ci`.

Rollback point: remove the new module, dependency, tests, and documentation changes; existing FSA, mode supervisor, and core PDO behavior remain independently usable.
