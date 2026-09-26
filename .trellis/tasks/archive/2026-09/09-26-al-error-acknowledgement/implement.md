# Implementation Plan

## Ordered Work

- [x] Add ESC Configuration constants and scan capability phase/record field.
- [x] Add scan unit tests for normal, Device Emulation, short response, and
      timeout behavior.
- [x] Add AL acknowledgement policy, phases, immutable fault record, and
      compatibility-preserving start API.
- [x] Add AL unit tests for disabled policy, successful acknowledgement,
      acknowledgement timeout, and first-error retention.
- [x] Wire scanned capability and policy into Startup, add fixed diagnostic
      record, and clear it on restart.
- [x] Update Startup simulations for both acknowledgement policies and Linux
      simulations for the added scan request.
- [x] Update exports, PRD/requirements/capability claims, and Trellis backend
      safety guidance.
- [x] Run focused tests, format, Clippy, `no_std` checks, full CI, BPF, HIL, and
      Zenoh gates.
- [x] Commit implementation, archive the task, update the journal, and push
      `main` to `ra1mj/ESOP`; verify the resulting GitHub Actions run.

## Validation Commands

```sh
cargo test -p esop-ethercat-core al:: scan:: startup::
cargo test -p esop-ethercat-linux-port --test scheduled_domains
cargo test -p esop-product-config
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo check -p esop-ethercat-core --target aarch64-unknown-none
cargo check -p esop-product-config --target aarch64-unknown-none
make ci
make bpf CLANG="$HOME/.local/opt/clang14/usr/bin/clang-14"
make test-hil
make test-zenoh
```

## Risk And Review Gates

- Adding one scan action changes every simulator that hard-codes the scan
  sequence; review all Startup-driving tests and Linux response fixtures.
- Never infer Device Emulation from ESC type or product identity; only the exact
  `0x0141[0]` response selects policy.
- Inspect every AL Control payload generated under Device Emulation and assert
  bit 4 is clear.
- Preserve the first AL error even when acknowledgement later times out or
  returns malformed data.
- Do not allow successful acknowledgement to release Startup or resume the
  transition automatically.
