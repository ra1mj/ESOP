# Error Handling

> Errors are explicit, typed, and propagated without partial publication.

## Overview

Public fallible operations return `Result<T, E>` with a crate-local enum.
Errors describe the rejected invariant (`ImageBounds`, `InvalidEntry`,
`PermitError`, and so on) and are normally passed to the caller with `?` or
`map_err`. Real-time code must not panic, allocate, sleep, or log to report an
error.

## Error Types

Use `#[derive(Clone, Copy, Debug, Eq, PartialEq)]` for fixed-size error enums.
Wrap lower-layer errors explicitly rather than erasing them. For example,
`Cia402PdoError::Pdo(PdoError)` preserves the core PDO failure while
`LifecycleError::Permit(...)` preserves the guard boundary.

## Error Handling Patterns

Validate all inputs before mutating caller-owned state. Transactional builders
stage into fixed arrays and publish only after every check succeeds. A write
path should look like:

```rust
self.preflight_output(image, fields)?;
self.write_signed(field, image, value)?;
```

Use `map_err` at crate boundaries, for example
`entry.read_unsigned(image).map_err(Cia402PdoError::Pdo)`.

## API Error Responses

There is no HTTP/API response layer. Library callers receive the typed error
directly; Linux examples may print it at the process boundary.

## Common Mistakes

- Mutating an image or registry entry before all bounds and capacity checks.
- Replacing a precise lower-layer error with a string or `Box<dyn Error>` in
  `no_std` code.
- Using `unwrap` in production paths; existing `unwrap` calls are test/example
  setup and assertions.

## EtherCAT AL Faults

Read Device Emulation only from the exact ESC Configuration `0x0141[0]`
response. A short, stale, timed-out, or bad-WKC capability read is a scan
failure; never choose a permissive acknowledgement policy by default.

Freeze the first AL Error Indication before attempting acknowledgement. Normal
ESCs may receive the observed AL state plus bit 4 through the bounded control
request path, but clearing the indication does not retry or complete the failed
transition. Device Emulation slaves must never receive bit 4 because AL Control
is mirrored into AL Status. A later acknowledgement timeout or malformed
response may become the terminal error, but it must not replace the original
slave position, requested/actual state, status code, or acknowledgement stage.
