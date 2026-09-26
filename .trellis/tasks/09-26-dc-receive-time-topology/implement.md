# Implementation Plan

## 1. Extend scan evidence

- [x] Add shared four-port and DL-status constants/types.
- [x] Add receive-time and Data Link Status phases with exact action metadata.
- [x] Decode four little-endian timestamps and all DL status flags.
- [x] Update scan/startup fixtures for DC, delay-only, and non-DC sequencing.

Validation:

```bash
cargo test -p esop-ethercat-core scan
```

## 2. Build fixed-capacity physical topology

- [x] Add public immutable topology slave/port/error types in `dc.rs`.
- [x] Implement stack-based 3,1,2 preorder construction and position lookup.
- [x] Reject capacity, duplicate, overrun, and unreachable-record inputs before
      returning a projection.
- [x] Cover linear and branched adjacency plus malformed topology.

Validation:

```bash
cargo test -p esop-ethercat-core dc_topology
```

## 3. Calculate propagation delays

- [x] Implement first-downstream-DC lookup and connected-port RTT sums.
- [x] Publish symmetric measurable DC edges using wrap-aware timestamps and
      checked aggregate arithmetic.
- [x] Walk the DC graph from an optional reference and publish cumulative
      transmission delays.
- [x] Cover delay-only/mixed topologies, wrap, non-first reference,
      incomplete evidence, underflow, and overflow.

Validation:

```bash
cargo test -p esop-ethercat-core dc_topology
```

## 4. Integrate Startup transactionally

- [x] Stage reference selection and topology in locals before publication.
- [x] Reject product-required slaves without cumulative delay before identity.
- [x] Expose immutable position-keyed topology and clear it on restart/fault.
- [x] Add a public request/RX round-trip regression for 16-byte receive times.

Validation:

```bash
cargo test -p esop-ethercat-core startup
cargo test -p esop-ethercat-core --test cycle
```

## 5. Sync requirements and qualification boundary

- [x] Update README, software/master requirements, capability manifest, and
      backend product/DC contract.
- [x] Preserve limitations for register writes, synchronization, physical
      timing/authenticity, HIL, WCET, conformance, and functional safety.
- [ ] Run formatting, focused tests, no-std, full CI, BPF, and exact pushed-SHA
      GitHub Actions verification.

Validation:

```bash
cargo fmt --all -- --check
git diff --check
make no-std
make capability-manifest
make ci
make bpf
```

## Review Gates

- Receive-time WKC 0 is never accepted; only the System Time capability probe
  retains zero-or-one semantics.
- A delay-only DC slave still receives and publishes the 16-byte timestamp
  block.
- Generic RX/control code remains register-agnostic.
- Topology and reference are both absent after any candidate validation error.
- Optional incomplete delay is explicit `None`; required incomplete delay is a
  pre-identity Startup fault.
- Cyclic paths gain no allocation, wait, sleep, logging, or unbounded search.
- Documentation does not imply physical delay precision or complete DC setup.

## Risk And Rollback Points

- Scan sequencing affects many simulator fixtures; centralize responses by
  register/phase and avoid action-ordinal assumptions.
- Port-loop semantics can make all-zero fixtures look like multiple open
  branches; tests must use explicit DL status rather than accidental zeros.
- Timestamp wrap is protocol behavior, while aggregate arithmetic failures are
  faults; do not replace both with one unchecked operation.
- Startup publication must remain one transaction even though reference
  selection and topology construction are separate computations.
