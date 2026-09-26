# Implementation plan: bounded SII category stream discovery

## 1. Core stream reader

- [x] Add the standard category start constant and stream request/phase/
      progress/error contracts.
- [x] Add a crate-private `SiiBlockReader` continuation path that preserves
      token/datagram cursors while retaining existing public-start behavior.
- [x] Implement fixed-capacity header/payload acquisition, absolute deadline,
      end-marker and no-partial-publication semantics.
- [x] Add request-pool enqueue, accept and timeout routing.

## 2. Atomic projection

- [x] Add `SiiConfigurationCandidate::apply_completed_stream` with
      caller-owned scratch.
- [x] Preserve explicit PDO signedness in both fixed-block and stream
      discovery finalization.
- [x] Add `SiiStreamDiscoveryController` without changing the exact-block
      controller.
- [x] Export all new public contracts from the core crate.

## 3. Verification

- [x] Add focused reader tests for multi-category success, unknown and
      zero-length categories, capacity/missing-end/address failures, cursor
      continuity, request ownership, stale responses and timeout.
- [x] Add controller tests for successful SM/PDO projection and projection
      failure without partial candidate publication.
- [x] Run focused core tests, `cargo fmt --all -- --check`, `git diff --check`,
      capability validation, `make ci`, `make bpf` and `make test-hil`.

## 4. Documentation and delivery

- [x] Update backend contract, README, PRD/requirements docs and capability
      manifest with the new evidence and remaining Startup/physical boundary.
- [x] Commit and push `main`, verify the matching GitHub Actions run, archive
      the task and record the Trellis journal.

## Verification record

- Local: focused core tests, full `esop-ethercat-core` tests, Clippy, no-std,
  `make ci`, `make bpf`, `make test-hil` and live `make test-zenoh` passed.
- Delivery commits: `2c82fa4` and `56def33` on `main`.
- Remote: GitHub Actions run `36267748560` for exact head
  `56def3327cb99541bbbf5db015cf23aac999f9eb` completed successfully, including
  Rust, Zenoh and every privileged eBPF runtime qualification job.

## Risk and rollback points

- Reusing a block reader by calling public `start` would reset action cursors
  and permit stale cross-category responses; only the internal continuation
  path may be used.
- Each category length is untrusted EEPROM data. Capacity and address checks
  must occur before issuing its first payload request.
- The total timeout must remain absolute across every continuation.
- The stream image is not evidence until END is accepted; the projected
  candidate is not evidence until finalization succeeds.
