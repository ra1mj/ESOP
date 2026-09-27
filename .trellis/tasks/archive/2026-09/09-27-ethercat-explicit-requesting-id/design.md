# Design: EtherCAT Explicit Requesting ID

## Boundaries

The core owns only a bounded register transaction state machine and Startup
sequencing. `esop-cfggen` owns ESI/product policy compilation.
`esop-product-config` owns static anti-tamper validation and profile attachment.
No runtime XML parsing or platform dependency enters the EtherCAT core.

## Core Contract

Add `explicit_id.rs` with:

- `RequestingIdPhase`: `Idle`, `WritingRequest`, `ReadingStatus`,
  `ReadingValue`, `Complete`, `Faulted`.
- `RequestingIdRequest`: station address, current INIT state, generation,
  operation deadline, request timeout.
- `RequestingIdAction`: immutable token/index/generation/address/operation,
  fixed two-byte payload/response shape, deadline, expected WKC 1.
- `RequestingIdProgress`: request written, polling, value read.
- `RequestingIdError`: busy/not-started/action identity/shape/WKC/deadline/state
  failures.
- `RequestingIdController`: one action at a time, first-fault retention,
  complete-only value publication, explicit restart reset.

Register constants live in `registers.rs`: ID Request and ID Loaded are bit 5.
The write payload is little-endian `INIT | 0x20`. Status polling reads exactly
two bytes and repeats only through caller-driven `next_action`; the value read
uses exactly two bytes at `0x0134`.

## Startup Integration

`StartupSlaveProfile` gains `expected_requesting_id: Option<u16>` and a builder.
Startup adds `ReadingRequestingId`, a controller instance, and per-position
verified values. After `finish_identity` records the ordinary SII identity:

- no expectation: follow the existing mailbox/SII/AL branch unchanged;
- expectation present: enter `ReadingRequestingId`;
- successful exact comparison: publish the verified value and continue through
  the same existing branch selector;
- mismatch/error: latch a typed Startup fault before any later action.

The existing production service scheduler already treats Startup as the highest
priority service and routes arbitrary `StartupAction` through the control pool.
The new action variant therefore extends the generic action accessors and
response dispatch without creating a parallel transport path.

## Generated Configuration

ESI model:

- `EsiDevice.requesting_id_supported: bool`
- only direct `Device/Info/IdentificationReg134` is accepted;
- value is a strict boolean; absence means false.

Product manifest:

```json
"identification": {
  "requesting_id": "0x0041"
}
```

The object defaults to empty. Generated slave data carries both
`requesting_id_supported` and `requesting_id: Option<u16>`. Generation rejects
an expectation without capability. Support without expectation remains
disabled. The fields participate in all deterministic artifacts and hashes.

`ProductSlaveConfig` mirrors both fields. Runtime validation repeats
enabled-implies-supported before generating `StartupSlaveProfile`, preventing a
tampered static artifact from bypassing the generator contract.

## Compatibility

The product schema name remains `esop.product.v1`; this is an optional additive
field whose absence preserves current behavior. ProcBuf ABI is unchanged because
Requesting ID is startup evidence, not a cyclic/shared-memory field.

## Failure And Rollback

All failures occur before AL activation and require explicit Startup restart.
No fallback to SII serial or best-effort continuation is allowed once the
product enables Requesting ID. Rollback is removal of the optional product
field and regenerated artifacts; the old runtime path remains intact.
