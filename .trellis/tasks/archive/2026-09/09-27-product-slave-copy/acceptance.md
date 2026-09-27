# Acceptance: Product-owned slave-to-slave copy plans

## Result

Accepted on 2026-09-27. Product manifests now own ordered semantic
slave-to-slave copy declarations, cfggen resolves and validates them into all
six deterministic artifacts, and runtime activation rebuilds one immutable
fixed-capacity plan set from its own activated Domain registry.

## Evidence

- The simulator product copies the left drive's `0x6064:0` actual position to
  the IO slave's vendor-specific `0x7010:1` mirror and writes `0x7011:1`
  quality without aliasing CiA 402 command fields.
- cfggen rejects unknown, ambiguous, wrong-direction, same-slave, width,
  datagram-coverage, capacity, strict-schema, and overlapping-target cases
  before artifact publication.
- Product activation rejects tampered PDO indices and excess plans before
  returning an `ActivatedProduct`; legacy products activate an empty plan set.
- The public generated-product test consumes a verified source Domain, checks
  the exact target frame payload and quality byte, and checks stale and bad-WKC
  fallback behavior.
- All six checked-in generated artifacts match fresh output byte for byte and
  carry configuration hash
  `e241ac19602f9adc68d206e726ed048fa3f915e34b04e6bd35a45af9774b6241`.

## Quality Gates

- `cargo test -p esop-ethercat-core --test slave_copy`
- `cargo test -p esop-cfggen --test generation`
- `cargo test -p esop-product-config --all-features`
- `make cfggen-runtime-example`
- focused all-target/all-feature Clippy with warnings denied
- `make ci`
- GitHub Actions `quality` run `36300420567` for
  `935a009281bdcb7201d9c99fa2e58b8fe71a209d`

All passed. This evidence does not prove physical slave behavior, response
provenance, timing/WCET, HIL interoperability, or functional safety. The cycle
owner still needs a later stable/double-buffered process-image publication
integration before copy plans can be applied automatically to long-lived TX
bindings.
