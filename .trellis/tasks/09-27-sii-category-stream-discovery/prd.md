# Bounded SII category stream discovery

## Goal

Provide an allocation-free control-plane path that discovers the variable-length
SII category stream from an ESC EEPROM until the standard end marker and
atomically projects the resulting SyncManager/RxPDO/TxPDO evidence through the
existing `SiiConfigurationCandidate` implementation.

This closes the fixed caller-supplied word-count gap in the current SII
discovery API and provides the direct prerequisite for Startup-owned live
SM/PDO verification. It does not claim that caller-delivered responses are
physically authentic.

## Background

- `SiiBlockReader<WORDS>` can read only a caller-declared contiguous word
  count.
- `SiiCategoryReader` and `SiiConfigurationCandidate` already parse and
  transactionally project complete category images.
- `SiiDiscoveryController` currently requires `SiiDiscoveryRequest.block` to
  contain the exact image word count, so it is not automatic SII category
  discovery.
- SII categories begin at a standard word address and are encoded as a two-word
  header followed by the declared payload, ending with category kind `0xFFFF`.

## Requirements

### R1. Fixed-capacity stream acquisition

- Add a `no_std`, allocation-free reader with caller-selected compile-time
  image capacity.
- Read two header words, then exactly the declared payload words, and repeat
  until the SII end category is observed.
- Preserve one absolute scan deadline, request-level deadlines, generation,
  monotonically advancing action token and datagram-index cursors across every
  internal header/payload read.
- Reuse the existing EEPROM register transaction and control-request-pool
  contracts instead of introducing a second wire implementation.

### R2. Fail-closed parsing boundary

- Reject capacities smaller than one category header, address overflow,
  category payloads that cannot fit the remaining image capacity, bad WKC,
  stale generation/token, malformed response length, EEPROM errors and
  deadline expiry with typed errors.
- Detect a missing end marker when another complete header cannot fit.
- Do not expose words, bytes, category count or a projected candidate as
  complete before the end marker has been accepted.
- Preserve the first terminal stream error until an explicit restart.

### R3. Atomic configuration projection

- Add a streaming discovery controller that owns the new reader and the
  existing `SiiConfigurationCandidate`.
- Reuse caller-owned scratch storage for word-to-byte conversion.
- Publish the candidate only after the complete image projects successfully;
  projection failure must leave no ready candidate.
- Preserve the discovery request's explicit PDO signedness policy when
  projecting both exact-block and stream images; existing default unsigned
  parsing remains unchanged.
- Keep the existing exact-block discovery API compatible.

### R4. Evidence and documentation

- Cover normal multi-category discovery, unknown-category preservation,
  zero-length category handling, capacity/end-marker failures, address
  overflow, request-pool ownership, stale response rejection, timeout and
  atomic projection failure.
- Update exports, requirements/PRD status, README and capability manifest with
  the exact software boundary.

## Acceptance Criteria

- [x] A fixed-capacity stream reader discovers a complete category image from
      the standard category start through `SII_CATEGORY_END` without heap
      allocation or blocking.
- [x] Every emitted action keeps bounded, non-reset token/datagram ownership
      across internal block boundaries and uses one absolute scan deadline.
- [x] Capacity, missing-end, address, WKC, generation, token, payload and
      timeout failures are typed and do not publish partial evidence.
- [x] A streaming discovery controller atomically publishes the existing
      SyncManager/PDO candidate and leaves it unavailable on projection error.
- [x] Existing fixed-range SII and Startup mailbox behavior remains compatible.
- [x] Focused tests, formatting, diff checks, workspace CI, BPF build and Linux
      simulator/HIL suites pass.
- [x] Changes are committed, pushed to `ra1mj/ESOP`, and the exact final
      GitHub Actions run succeeds.

## Out Of Scope

- Startup integration or generated-product comparison of the discovered
  candidate.
- FMMU logical-address allocation policy, DC category semantics or Status Bit
  mailbox discovery.
- Authenticating physical ESC responses, real-device interoperability, target
  WCET or physical HIL qualification.
