# Implementation plan

- [x] Add `crates/esop-ipc` to the workspace with separate frame, peer and Unix
      endpoint modules and no external runtime dependencies.
- [x] Implement the frozen 80-byte little-endian v1 header, bounded owned frame,
      explicit encode/decode and IEEE CRC-32 validation.
- [x] Add exhaustive codec tests for golden bytes, maximum payload and every
      malformed/truncated/corrupt input class.
- [x] Implement `PeerPolicy`/`PeerMonitor` identity, boot, replay, timestamp,
      age, timeout, restart and reconnect semantics with boundary tests.
- [x] Implement the nonblocking Unix datagram endpoint, exact source-path
      admission, one-byte oversize detection, stable OS error mapping and
      owned-path cleanup.
- [x] Add kernel-backed integration tests for bidirectional traffic, empty
      receive, absent/unexpected peer, oversized datagram and same-path peer
      restart with a new boot ID.
- [x] Add `docs/esop-ipc.md`; update FR-029/R3 status in the software PRD and
      plan, and add evidence-bound `unix_domain_ipc` capability metadata.
- [x] Update backend quality guidance with the cross-layer IPC contract and
      validate RT dependency trees remain free of `esop-ipc`/POSIX transport.
- [x] Run `cargo test -p esop-ipc`, focused Clippy/check, capability validation,
      formatting, `git diff --check`, dependency-tree checks and full `make ci`.
- [ ] Review trust boundaries and version compatibility, commit, push, track
      the matching GitHub Actions run, then archive and record the task.
