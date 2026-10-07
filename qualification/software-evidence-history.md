# Historical Software Acceptance Evidence

These are dated September 2026 records preserved during repository cleanup.
They retain their original findings, citations and qualification limits.
Statements about missing features or future work describe that historical
snapshot, not the current implementation. Current source, backend specs and
the qualification manifests take precedence. No new HIL or release claim
is established by copying these records. Original bytes remain available
at the pinned source links; only heading levels and a local compiler path
are normalized below.

<a id="09-27-esc-watchdog-configuration-acceptance-md"></a>

## 09-27-esc-watchdog-configuration/acceptance.md

[Original record](https://github.com/ra1mj/ESOP/tree/c7d43e51ed0a5e12107bad8f55267087801568fe/.trellis/tasks/archive/2026-09/09-27-esc-watchdog-configuration/acceptance.md) · SHA-256 `18fba25b098b668db20d246a234ea60b888f30407a63245e718fee68a6cf2dd5`

### Acceptance: ESC watchdog configuration and readback

#### Result

Accepted on 2026-09-27. The implementation makes optional per-slave ESC
watchdog divider and process-data interval intent deterministic product data,
programs each configured standard register through the bounded control path,
verifies it with an independent exact readback, and holds the PREOP barrier
until the complete watchdog plan succeeds.

#### Evidence

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

#### Quality Gates

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

<a id="09-27-product-slave-copy-acceptance-md"></a>

## 09-27-product-slave-copy/acceptance.md

[Original record](https://github.com/ra1mj/ESOP/tree/c7d43e51ed0a5e12107bad8f55267087801568fe/.trellis/tasks/archive/2026-09/09-27-product-slave-copy/acceptance.md) · SHA-256 `86c0915b13c72a1ee318601d1288a315c7f41d1960cecd36f18070ee494a31a0`

### Acceptance: Product-owned slave-to-slave copy plans

#### Result

Accepted on 2026-09-27. Product manifests now own ordered semantic
slave-to-slave copy declarations, cfggen resolves and validates them into all
six deterministic artifacts, and runtime activation rebuilds one immutable
fixed-capacity plan set from its own activated Domain registry.

#### Evidence

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

#### Quality Gates

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

<a id="09-27-sii-dc-descriptor-verification-acceptance-md"></a>

## 09-27-sii-dc-descriptor-verification/acceptance.md

[Original record](https://github.com/ra1mj/ESOP/tree/c7d43e51ed0a5e12107bad8f55267087801568fe/.trellis/tasks/archive/2026-09/09-27-sii-dc-descriptor-verification/acceptance.md) · SHA-256 `4add8fda8864078074e11280bc2d60852c1f9cd0f3bd2213dff2fa44a7efca1d`

### Acceptance: SII DC descriptor verification

#### Result

Accepted on 2026-09-27. The implementation adds strict borrowed SII Strings
and fixed 24-byte DC descriptor parsing, strict ordered ESI `Dc/OpMode`
selection, generated runtime expectations, and one-pass pre-AL Startup
verification with transactional structural/DC evidence publication.

#### Evidence

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

#### Quality Gates

- `cargo test -p esop-ethercat-core sii --lib`
- `cargo test -p esop-cfggen --all-targets`
- `make cfggen-runtime-example`
- `make ci`
- `make bpf`
- `git diff --check`

All passed locally. Physical response authenticity, common SYNC start-time
calculation, topology-wide SYNC register programming, HIL, conformance and
functional-safety qualification remain explicitly out of scope.
