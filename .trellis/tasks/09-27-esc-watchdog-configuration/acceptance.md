# Acceptance: ESC watchdog configuration and readback

## Result

Accepted on 2026-09-27. The implementation makes optional per-slave ESC
watchdog divider and process-data interval intent deterministic product data,
programs each configured standard register through the bounded control path,
verifies it with an independent exact readback, and holds the PREOP barrier
until the complete watchdog plan succeeds.

## Evidence

- Core tests cover empty and optional-field plans, product-order traversal,
  exact little-endian `0x0400/2` then `0x0420/2` write/read ordering,
  complete-only evidence, control-pool ownership, typed substitution/readback/
  WKC/timeout/deadline failures, and explicit restart.
- cfggen tests reject unknown, empty, and zero-valued declarations and prove
  deterministic normalized JSON, inventory, C, Rust, and configuration hash
  outputs.
- Product-config tests prove generated-order position, station, and value
  propagation, invalid static-data rejection, and the unconfigured empty plan.
- Production scheduler and Linux simulation tests prove PDO-to-watchdog-to-
  mapping priority, cross-cycle request ownership, exact completion routing,
  typed faulting, and PREOP release only after watchdog completion.
- Legacy products and startup masks without a watchdog requirement retain
  their prior service traffic and completion behavior.

## Quality Gates

- `cargo test -p esop-ethercat-core --no-fail-fast`
- `cargo test -p esop-cfggen --no-fail-fast`
- `cargo test -p esop-product-config --no-fail-fast`
- `cargo test -p esop-lifecycle-guard --no-fail-fast`
- `cargo test -p esop-ethercat-linux-port --test scheduled_domains --no-fail-fast`
- `make cfggen-runtime-example`
- `make capability-manifest`
- `make ci`
- GitHub Actions `quality` run `36298255226` for
  `e7c8e8e7f1189ce273748afcc843f720926df713`

All passed. Physical watchdog expiration, response authenticity, target
timing/WCET, HIL, interoperability, ETG conformance, and functional-safety
qualification remain explicitly out of scope.
