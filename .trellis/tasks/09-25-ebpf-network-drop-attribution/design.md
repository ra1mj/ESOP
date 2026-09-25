# Design: eBPF EtherCAT network drop attribution

## Data flow

```text
skb:kfree_skb
  -> typed protocol/reason read
  -> CO-RE skb dev/ifindex read with skb_iif fallback
  -> EtherType + optional ifindex filter
  -> bounded {cpu, ifindex} window state
  -> threshold-crossing 96-byte RuntimeEvidence
  -> Aya decoder
  -> IncidentCorrelator + transport-risk CycleContext
  -> HOST_NIC_DROP
```

## Kernel contracts

- `esop_context` adds `network_ifindex`, `network_protocol`, and
  `network_drop_window_ns`. The C and Rust definitions remain fixed-size and
  have explicit size assertions.
- `ESOP_NETWORK_DROPS` is a hash map with 256 entries. Its key is CPU plus
  ifindex; its value is `window_start_ns`, count, and the latest drop reason.
- A new window starts for a missing/expired entry. The event that makes
  `count == threshold` emits once; later drops in the same window only update
  counters. Window expiry permits a later threshold event.
- Map update failure increments `lost_events`. A protocol match with no usable
  ifindex increments `network_unattributed` and returns without incident
  evidence.
- Network evidence sets PID/TID to zero because softirq current-task identity
  is not packet ownership. It carries ifindex and a saturated one-byte drop
  reason detail.

## ABI and userspace

- The event remains exactly 96 bytes. The final byte changes from unused
  `reserved` to kind-specific `detail`; all non-network producers write zero.
- `KernelStats` adds `network_drops`, `network_unattributed`, and
  `network_threshold_events`.
- `RuntimeConfig` defaults to EtherCAT protocol `0x88A4`, any interface,
  threshold 1, and a 1 ms window. A dedicated update method changes all
  network filter/window fields atomically in the context map.
- Existing `update_tracking` remains compatible for PID/scheduler tracking and
  the network count threshold. The dedicated network method owns protocol,
  ifindex, and window changes.

## Correlation and merging

- `NetworkDrop` remains soft evidence. It becomes `HOST_NIC_DROP` only when
  its count reaches threshold and the current `CycleContext` reports deadline,
  WKC, or DC risk in the correlation window.
- Incident merging continues to partition nonzero network interfaces so drops
  from different interfaces cannot merge into one incident.
- The raw evidence retains the drop reason detail; the incident top level
  remains ABI-compatible and points to the bounded raw evidence array.

## Compatibility and rollback

- `kfree_skb` remains optional in the default required mask. Missing tracepoint
  fields or BTF relocation/load failure produces a reduced attach mask.
- No ProcBuf, Protobuf, MLG, EtherCAT core, or motion-control ABI changes are
  required.
- Rollback removes the new context/stat fields and map while preserving the
  existing evidence discriminant and optional attach bit.

## Qualification boundary

CO-RE compilation proves source/BTF compatibility for the CI header only. Real
kernel load, verifier behavior, NIC queue pressure, drop injection, and overhead
remain EBPF-004/EBPF-010 target-environment work.
