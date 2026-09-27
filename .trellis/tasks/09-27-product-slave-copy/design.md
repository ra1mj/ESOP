# Design

## Data flow

```text
product.json slave_copies
  -> cfggen stable PDO lookup (slave, index, subindex)
  -> activated generator DomainRegistry + SlaveCopyPlan::build
  -> normalized JSON / C / Rust / config SHA-256
  -> StaticProductConfig resolved PDO indices
  -> runtime DomainRegistry activation + SlaveCopyPlan::build
  -> ActivatedProduct::slave_copy_plans()
  -> cycle owner apply after RX and before target TX
```

The stable manifest identity is semantic. Generated runtime data uses PDO array
indices only after cfggen has resolved and validated the semantic references.
Runtime activation retains only the fixed-capacity source, target, and quality
`PdoEntryHandle`s referenced by copy declarations, activates the registry, and
rebuilds the plans from those handles. This keeps the generated artifact
compact, preserves `PDOS` as a per-Domain capacity, and ensures offsets and
datagram coverage are never trusted without revalidation.

## Core plan set

Add `SlaveCopyPlanSet<const PLANS: usize>` to `esop-ethercat-core`. It owns a
fixed array, count, ordered push, and overlap validation. Its public slice is
immutable after product activation. `SlaveCopyPlan` supplies read-only width and
target-range metadata needed by metrics and set validation; execution semantics
stay in the existing plan methods.

Overlapping target payload or quality bytes are rejected because plan order
must not decide the transmitted value. Capacity failure and overlap failure do
not mutate the set.

## Generator validation

`ProductManifest.slave_copies` defaults to empty and remains strict. Each entry
has a unique validated name, source/target/quality references, and `u8`
`invalid_fill`.

Cfggen resolves references only within selected PDOs. It requires exactly one
match and then calls the core builder after registry activation. The generated
record carries semantic identities, resolved PDO indices, payload width, and
fallback. The record participates in the normalized semantic hash and every
product-facing artifact.

`copy_bytes_per_cycle` sums payload widths for copies whose target Domain is
due on each schedule tick, then reports the maximum. It excludes ordinary input
bytes and the one-byte quality write.

## Runtime activation

`StaticProductConfig` gains a borrowed slice of `ProductSlaveCopyConfig`.
`ActivatedProduct` owns `SlaveCopyPlanSet<MAX_PRODUCT_SLAVE_COPIES>` and exposes
an immutable accessor. The fixed maximum avoids adding a new const generic to
every product type and call site.

Activation validates every resolved PDO index, rebuilds every plan from the
active registry, and maps precise lower-layer errors into
`ProductActivationError`. No activated product is returned on failure.

## Example and compatibility

The simulator IO RxPDO gains a 32-bit vendor-specific mirror and an unsigned
8-bit quality field. The manifest copies drive-left `0x6064:0` into those IO
fields. It does not overwrite an axis Controlword, mode, or target.

Products without `slave_copies` generate an empty static array and activate an
empty plan set. Existing `SlaveCopyPlan` callers remain source compatible.

## Evidence boundary

Host simulation proves deterministic generation, activation, frame bytes,
freshness, WKC degradation, and stale fallback. It does not prove physical
slave mapping/readback, WCET, a two-cycle hardware bound, or functional safety.
