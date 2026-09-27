# Implementation plan: DC sync-window runtime monitoring

## Phase 1: Core configuration and monitor

- [x] Add `DcSyncWindowConfig`, topology-derived expected-WKC validation,
      sample/monitor types, loss/recovery counters, and focused unit tests.
- [x] Export the additive contracts without changing legacy constructors.

## Phase 2: Atomic cyclic DC exchange

- [x] Extend `DcCyclicSync` with optional BRD planning, zeroed request payload,
      same-generation staging, response-order independence, atomic commit, and
      missing-response observation.
- [x] Add exact shape/WKC/index/generation/bounds tests and prove legacy one-
      datagram behavior remains unchanged.

## Phase 3: Scheduler and lifecycle integration

- [x] Extend scheduled-domain DC TX/RX preflight to one or two plans and reject
      all Domain/DC/control index aliases before mutation.
- [x] Change lifecycle and ProcBuf quality projection to use the combined DC
      lock predicate while preserving current-cycle freshness requirements.
- [x] Add shared Domain/DC/control, lock-loss, missing-window, and recovery
      integration tests, including a public Linux simulated-port path.

## Phase 4: Documentation and verification

- [x] Update README, EtherCAT requirements, software PRD/architecture docs,
      lifecycle docs, backend quality/product specs, and capability manifest.
- [x] Run focused tests, format, workspace all-feature checks/tests/Clippy,
      AArch64 `no_std`, `make ci`, `make bpf`, capability validation, Trellis
      validation, and `git diff --check`.
- [ ] Record acceptance evidence, commit and push implementation, require exact
      SHA GitHub Actions success, archive the task, journal the session, push
      housekeeping commits, and require final exact-SHA CI success.

## Risk And Rollback Points

- Preserve atomic current-cycle publication when responses arrive in either
  order or only one response arrives.
- Keep all index conflict checks before `prepare`; otherwise a rejected cycle
  could leave an owned generation without a legal RX finalizer.
- Do not reinterpret the BRD aggregate as per-slave attribution.
- Do not change ProcBuf layout in this increment; expose the aggregate through
  the core API and keep `dc_offset_ns` semantics unchanged.
- Keep monitoring separate from automatic DC register correction.
