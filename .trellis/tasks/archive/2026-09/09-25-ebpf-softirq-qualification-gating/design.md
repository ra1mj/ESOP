# Design: eBPF softirq qualification gating

## Root cause

The production BPF CPU/vector predicate is spatial: it admits every matching
softirq while the filter remains configured. The qualification treated that as
temporal isolation and kept vector 3 active from load through polling. A quiet
CPU sample reduces background traffic but cannot guarantee that no unrelated
`NET_RX` action occurs in that wider interval.

## Gate contract

Use `u32::MAX - 1` as a qualification-only closed vector. Linux softirq vectors
are small bounded identifiers, while `u32::MAX` remains the production
all-vector sentinel. No production API or ABI field changes are required.

Each phase follows this order:

```text
load selected CPU + closed vector
  -> verify attach pair and closed context
  -> project Healthy capability and risk cycle
  -> require empty BPF statistics
  -> read target CPU NET_RX before
  -> update filter to target CPU + vector 3
  -> verify active context
  -> send one UDP_SEGMENT GSO payload
  -> read target CPU NET_RX after
  -> update filter back to target CPU + closed vector
  -> verify closed context
  -> require NET_RX delta = 1
  -> receive exactly 54 datagrams
  -> poll and require exact record/incident/statistics
```

The send/counter operation is captured as a result before it is returned. The
close update is always attempted after a successful open, including when send
or counter sampling fails. A close failure takes precedence because proceeding
with an open observation window invalidates isolation.

## Evidence contract

The existing report remains a closed flat JSON object. Add:

- `interrupt_gate_closed_vector`
- `calibration_filter_vector_before`
- `calibration_filter_vector_active`
- `calibration_filter_vector_after`
- `filter_vector_before`
- `filter_vector_active`
- `filter_vector_after`

The validator requires the constant closed vector for both before/after fields
and vector 3 for both active fields. Existing exact counts and all semantic
checks remain unchanged.

## Compatibility

- Production `RuntimeConfig`, `KernelContext`, BPF maps, evidence records, and
  attach behavior are unchanged.
- `BpfRuntime::update_interrupt_filter` is reused; no second filter mechanism is
  introduced.
- The report schema is qualification-internal and version 1 remains acceptable
  because consumers are the co-versioned strict validator and CI artifact.

## Failure and rollback

Any open/update/send/counter/close mismatch returns before report publication.
The runner already deletes stale final/temp output and writes only after both
phases pass. Rollback reverts the qualifier, validator/tests, and documentation;
it does not touch the production BPF ABI.
