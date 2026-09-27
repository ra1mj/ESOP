# Acceptance: SII DC descriptor verification

## Result

Accepted on 2026-09-27. The implementation adds strict borrowed SII Strings
and fixed 24-byte DC descriptor parsing, strict ordered ESI `Dc/OpMode`
selection, generated runtime expectations, and one-pass pre-AL Startup
verification with transactional structural/DC evidence publication.

## Evidence

- Core parser tests cover multiple descriptors, signed fields, malformed
  lengths, missing Strings, invalid indices, reserved bytes, unknown and
  duplicate names.
- cfggen tests cover ordered multi-mode parsing, signed factor widths,
  malformed nesting, required/non-required selection policy, unknown and
  duplicate modes, direct SYNC1 rejection, generated fields and deterministic
  semantic/configuration hashes.
- Product-config tests prove policy invariants and exact expectation
  propagation before Startup mutation.
- Startup tests prove the existing single category-stream pass is reused,
  mismatch faults before AL, structural and DC evidence publish together,
  and restart clears evidence.
- The dual-axis simulator selects `DcSync` for both drives; the IO slave keeps
  no DC descriptor expectation. Generated Rust and C artifacts compile and
  match the checked-in golden output.

## Quality Gates

- `cargo test -p esop-ethercat-core sii --lib`
- `cargo test -p esop-cfggen --all-targets`
- `make cfggen-runtime-example`
- `make ci`
- `make bpf`
- `git diff --check`

All passed locally. Physical response authenticity, common SYNC start-time
calculation, topology-wide SYNC register programming, HIL, conformance and
functional-safety qualification remain explicitly out of scope.
