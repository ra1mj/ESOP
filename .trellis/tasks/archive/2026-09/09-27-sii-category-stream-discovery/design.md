# Design: bounded SII category stream discovery

## Data flow

```text
SiiCategoryStreamRequest
  -> SiiCategoryStreamReader<IMAGE_WORDS>
       -> SiiBlockReader<2> (header or one payload word)
       -> fixed [u16; IMAGE_WORDS] unpublished image
       -> SII_CATEGORY_END
  -> caller-owned byte scratch
  -> SiiConfigurationCandidate::apply_bytes
  -> SiiStreamDiscoveryController::candidate (Ready only)
```

## Reader contract

Add `sii_stream.rs` with:

- `SII_CATEGORY_START_WORD = 0x0040` as the standard category start.
- `SiiCategoryStreamRequest`, carrying station, optional explicit start word,
  generation and both total/request timeout inputs.
- `SiiCategoryStreamPhase`: `Idle`, `ReadingHeader`, `ReadingPayload`,
  `Complete`, `Faulted`.
- `SiiCategoryStreamProgress`: bounded progress plus category completion and
  final completion evidence.
- `SiiCategoryStreamError`: typed validation errors and wrapped
  `SiiBlockError`/`ControlError`.
- `SiiCategoryStreamReader<IMAGE_WORDS>`.

The stream reader stores the accepted image in words. Its internal
`SiiBlockReader<2>` reads each two-word header and then each payload word. This
avoids a second maximum-category-sized buffer while retaining the existing
EEPROM action, WKC, timeout and request-pool implementation.

`SiiBlockReader` gains a crate-private continuation entry that starts a new
range after `Complete` while preserving its next token and datagram-index
cursors. The public `start` behavior remains unchanged. Each continuation is
given the remaining duration to the original absolute stream deadline, so a
long category sequence cannot refresh the total timeout.

The header is capacity-checked before any payload request is started. The end
header is retained in the image so the existing `SiiCategoryReader` remains the
single parser. Public `words`/`copy_bytes` access succeeds only in `Complete`.

## Projection controller

Keep the existing exact-block `SiiDiscoveryController` unchanged. Add
`SiiStreamDiscoveryController` alongside it, using the same high-level phases:

1. `start` resets stream and unpublished candidate.
2. `next_action`/`enqueue_pending`/`accept`/`timeout` delegate to the stream.
3. End-marker completion moves to `Projecting`.
4. `finalize` uses caller-owned scratch and a staged candidate copy.
5. Only successful projection moves to `Ready` and exposes `candidate()`.

`SiiConfigurationCandidate::apply_completed_stream` mirrors
`apply_completed_block`; it copies bytes into caller scratch and then reuses
the existing transactional `apply_bytes` path. Signedness-aware variants feed
the controller request's existing `signed` flag into every PDO category. The
legacy unsigned methods delegate with `false`.

## Compatibility

- No existing public struct literal or controller method changes.
- Fixed-block discovery remains available for known EEPROM ranges.
- Startup mailbox verification continues using its exact five-word reader.
- New types are re-exported from `esop-ethercat-core`.

## Failure and rollback

- Stream errors latch `Faulted`; restart is the only recovery.
- An early `timeout` call keeps the current request pending, matching
  `SiiBlockReader` semantics. Expiry faults the stream.
- Projection uses a staged candidate and cannot publish a partial layout.
- If memory or API cost is unacceptable, the new module/controller can be
  removed without changing existing callers because integration is additive.

## Qualification boundary

Software evidence proves bounded acquisition and parsing of responses supplied
through the existing control path. It does not prove response source,
electrical behavior, EEPROM correctness, target timing or device
interoperability.
