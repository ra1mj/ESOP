# Type Safety

> No frontend type system is present; Rust type safety is documented by the
> backend crate conventions.

## Current Scope

The workspace uses Rust 2024 with `rust-version = 1.85`; there is no
TypeScript compiler or runtime validation library.

## Type Organization

Not applicable.

## Validation

Not applicable. Rust APIs use concrete structs/enums and explicit conversion
functions such as `OperatingMode::from_raw`.

## Common Patterns

Rust generics and const-capacity arrays are used where bounded storage matters;
see `PdoLayout<const ENTRIES: usize>` and `FixedEventQueue`.

## Forbidden Patterns

Do not introduce `any` or unchecked type assertions as a substitute for a
future frontend contract, and do not weaken Rust APIs with untyped strings.
