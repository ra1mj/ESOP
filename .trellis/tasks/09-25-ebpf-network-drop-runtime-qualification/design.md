# Design: eBPF network drop runtime qualification

## Qualification data flow

```text
privilege/tool preflight
  -> reserve unique veth names and create peer pair
  -> disable per-link IPv6, bring links up, read identities/counters
  -> pin fixture to one allowed CPU and open protocol-zero AF_PACKET socket
  -> production BPF attaches only skb:kfree_skb for EtherCAT + RX ifindex
  -> wrong-protocol and reverse-direction controls remain silent
  -> observe transport-risk cycle and verify zero formal baseline
  -> send four forward EtherCAT frames
  -> RX rx_dropped delta + exact BPF counters prove filtered drop path
  -> ringbuf decode -> HostNicDrop -> Degraded heartbeat
  -> detach/close -> delete veth -> verify sysfs cleanup
  -> atomic JSON report -> strict validator -> CI artifact
```

## Link and packet contract

- Generate two interface names from the parent PID, bounded to Linux's
  15-character interface-name limit. Reject either name when already present.
- Create one veth pair in the current network namespace with `ip link add`.
  The fixture owns both names and deleting the transmit end removes the pair.
- Write `1` to each disposable interface's
  `/proc/sys/net/ipv6/conf/<name>/disable_ipv6` when present, then bring both
  links up. No host-wide sysctl is changed.
- Read ifindexes and MACs from sysfs and require distinct positive ifindexes
  and valid unicast six-byte MAC addresses.
- Pin the process to the first CPU in its inherited affinity set before BPF
  load and packet injection. Restore the inherited mask during cleanup.
- Create one `AF_PACKET/SOCK_RAW|SOCK_CLOEXEC` socket with protocol zero.
  Each send supplies `sockaddr_ll` with the transmit ifindex and network-order
  EtherType. The frame contains destination peer MAC, source local MAC,
  EtherType, and deterministic payload, and is at least the minimum Ethernet
  frame size excluding FCS.

## Control and formal injection contract

- Set the runtime filters to receive ifindex and EtherType `0x88a4`, threshold
  four, and a one-second aggregation window.
- Forward control: send one frame through TX to RX with EtherType `0x88a5`.
  Reverse control: send one EtherCAT frame through RX to TX. Poll boundedly and
  require no network-drop, emission, loss, record, or incident.
- Controls can increment unfiltered interface counters; capture the formal
  `rx_dropped` baseline only after controls. BPF counters must still be exactly
  zero because both controls fail one configured filter.
- Record one cycle with nonzero cycle/transition identity and WKC risk before
  the formal injection.
- Send exactly four EtherCAT frames TX to RX in one bounded burst. Poll until
  the threshold record arrives, then require no second record during a quiet
  drain interval.
- Require the RX `rx_dropped` delta to be at least four. Exact BPF counts remain
  four because protocol and ifindex filters isolate the formal frames.

## Runtime and correlation contract

- The runtime enables and requires only `ATTACH_NETWORK_DROP`. The hosted load
  gate also verifies that the vendored `kfree_skb.reason` type preserves the
  target `enum skb_drop_reason` BTF kind so Aya can apply its `FIELD_SIGNED`
  relocation. Decoder, statistics, correlator, and health policy remain
  unchanged.
- The BPF aggregation key is `{cpu, ifindex}`. CPU affinity plus synchronous
  veth receive processing makes one deterministic key; the emitted CPU must
  equal the selected CPU.
- The threshold record carries count/observed/threshold four, elapsed duration
  within the one-second window, receive ifindex, and kernel drop reason in
  `detail`. PID/TID remain zero for this kernel-network event.
- Network-drop classification requires transport risk in the correlated cycle.
  One record produces Error `HostNicDrop`, action `ControlledStop`, confidence
  75. Runtime health changes from Healthy to Degraded with fault `0x45422001`.

## Evidence and statistics contract

- Evidence domain/kind/severity are KernelNetwork/NetworkDrop/Error. Evidence
  ID and timestamp are nonzero; IRQ is zero; CPU, ifindex, cycle, transition,
  values, duration, count, and detail satisfy the exact contracts above.
- The incident matches evidence identity and fields, contains one evidence
  item, and reports no dropped evidence or incident.
- Exact final BPF statistics are four network drops, zero unattributed drops,
  one network threshold event, one emitted event, and zero loss. All unrelated
  event counters remain zero.

## Cleanup and report publication

- Delete stale final and temporary reports before setup.
- RAII cleanup drops the BPF runtime and packet socket before deleting the
  transmit veth. It then verifies both `/sys/class/net/<name>` paths disappear
  within a bounded deadline and restores the original CPU affinity.
- Explicit success cleanup records successful runtime detach, socket close,
  link deletion, sysfs disappearance, and affinity restoration. Any cleanup
  failure prevents report publication.
- Only after cleanup succeeds, write the complete closed-schema report to a
  same-directory temporary path and rename it atomically.
- The report records link identities/configuration, selected CPU, control and
  formal frame counts, drop-counter baselines/deltas, runtime configuration,
  capability snapshot, exact BPF/poll statistics, full incident/evidence
  fields, health snapshots, and cleanup outcomes.

## Compatibility and safety

- This task adds the qualification harness and, if the target verifier exposes
  a CO-RE portability defect, may minimally align the vendored tracepoint field
  type/read with the target kernel. Production maps, fixed ABIs, runtime APIs,
  attachment shape, and incident policy remain unchanged.
- The example is Linux-only and uses existing `libc` support. Production
  crates gain no dependency.
- BPF/Rust compilation and report validation remain unprivileged. Only the
  final prebuilt fixture is elevated for veth, raw-packet, and BPF operations.

## Operational limits and rollback

- Hosted CI proves one controlled virtual-veth unhandled-EtherType receive-drop
  chain. It does not qualify physical NICs, drivers, NAPI, XDP, qdisc, queue
  pressure, real EtherCAT devices, production kernels, overhead, or WCET.
- Rollback removes the example, runner, validator/tests, Make/CI targets, docs,
  capability-evidence entry, and the paired enum-kind tracepoint field/read
  alignment if the network-drop program is removed.
