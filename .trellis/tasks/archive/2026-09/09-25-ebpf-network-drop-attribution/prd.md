# eBPF EtherCAT network drop attribution

## Goal

Complete the bounded Linux kernel-to-incident path for EtherCAT skb drops so
ESOP can identify a thresholded per-interface drop window that overlaps a
degraded EtherCAT cycle.

## Background

The current `skb:kfree_skb` program emits one generic event per matching
current process. That is not a reliable network attribution mechanism because
skb release commonly runs in softirq context, the event carries no interface
identity or drop reason, and the configured threshold is not aggregated in the
kernel. FR-048 and EBPF-004 require interface-scoped evidence that can be
correlated with an EtherCAT WKC/timeout window without flooding the ring buffer.

## Requirements

- Observe `skb:kfree_skb` using its typed tracepoint context and preserve the
  kernel drop reason as a saturating kind-specific detail value.
- Restrict evidence to a configured EtherType, defaulting to EtherCAT
  `0x88A4`. Do not use `tracked_pid` as the network filter.
- Resolve the skb network interface from `skb->dev->ifindex`, falling back to
  `skb_iif`. A configured nonzero ifindex must match exactly.
- Do not emit `HOST_NIC_DROP` evidence when the interface cannot be resolved.
  Count unresolved protocol-matching drops in per-CPU statistics instead.
- Aggregate drops in a fixed-capacity BPF map keyed by CPU and ifindex. The map
  and each entry's time window must have explicit bounds.
- Emit at most one fixed 96-byte evidence record when a window first reaches
  the configured count threshold. Preserve `boot_id`, `agent_epoch`,
  `cycle_seq`, `transition_seq`, CPU, ifindex, count, threshold, elapsed window,
  and drop reason.
- Expose network ifindex, EtherType, count threshold, and window duration
  through `RuntimeConfig`, the kernel context ABI, and a runtime update API.
- Extend per-CPU statistics with total attributed drops, unresolved drops, and
  emitted threshold windows, using saturating userspace aggregation.
- Correlate a threshold event into `HOST_NIC_DROP` only when it overlaps an
  existing transport-risk cycle. An event outside that cycle/window is
  evidence only and must not be presented as a root cause.
- Keep the network tracepoint optional in the default required attach mask so
  older kernels can report reduced capability without breaking the existing
  required observation path.
- Update the PRD status, runtime observability design, capability manifest,
  README, and backend quality contract without claiming target-kernel packet
  injection qualification.

## Acceptance Criteria

- [x] The BPF bundle contains a bounded per-CPU/per-interface drop-window map.
- [x] EtherCAT protocol filtering and optional exact-ifindex filtering are
      independent of the current PID/TID.
- [x] Unresolved interfaces cannot emit threshold evidence and are visible in
      statistics.
- [x] The fixed 96-byte evidence ABI decodes interface identity and the drop
      reason detail without enum transmutation.
- [x] Runtime configuration rejects zero protocol, threshold, or window values.
- [x] Unit tests prove `HOST_NIC_DROP` requires both threshold evidence and a
      correlated transport-risk cycle, and preserves the interface/detail.
- [x] Unit tests cover context preservation, map/stat ABI sizes, saturation,
      and network configuration updates.
- [x] `make ci` passes locally.
- [x] GitHub Actions builds the real CO-RE BPF object successfully.

## Non-Goals

- Do not claim EBPF-004 target-kernel queue/drop injection qualification from
  compile-time or unit-test evidence alone.
- Do not add driver-specific tracepoints, NAPI latency, queue depth, TX latency,
  packet payload parsing, or raw-port packet capture in this increment.
- Do not treat every skb free as an EtherCAT drop; protocol filtering remains
  mandatory.
- Do not make eBPF evidence a direct MLG, PDO, controlword, or motion-permit
  writer.

## Technical Notes

- The current kernel tracepoint records skb address, protocol, and drop reason.
  Interface identity is read from the skb through a CO-RE kernel read helper.
- Kernels whose tracepoint/BTF shape cannot load the optional program remain
  degraded rather than blocking the existing scheduler/process observation
  baseline.
