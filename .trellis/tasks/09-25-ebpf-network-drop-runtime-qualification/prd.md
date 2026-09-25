# eBPF network drop runtime qualification

## Goal

Qualify the existing FR-048/EBPF-004 network-drop attribution path on a
privileged hosted Linux kernel with an isolated virtual Ethernet link. The
evidence must prove that the production CO-RE object can load and require
`skb:kfree_skb`, filter on the exact EtherCAT EtherType and receive ifindex,
observe a controlled unhandled-protocol receive drop, emit one threshold
record, correlate it with transport risk as `HOST_NIC_DROP`, and project the
incident into degraded observer health.

## Background

The repository already attaches to `skb:kfree_skb`, reads the host-order
protocol and kernel drop reason, resolves the skb device ifindex, aggregates
drops per CPU and ifindex, emits a fixed 96-byte `KernelNetwork/NetworkDrop`
record at the configured threshold, and classifies correlated records as
Error `HOST_NIC_DROP` with `ControlledStop`. Source tests and CO-RE compilation
do not establish real verifier/load behavior, tracepoint availability, packet
injection, protocol/ifindex filtering, threshold aggregation, ringbuf delivery,
or observer-health projection. In particular, the hosted verifier/load gate
must reject unresolved compiler CO-RE pseudo-helper calls that source syntax
and object compilation cannot detect.

## Requirements

- Add a Linux-only `network_drop_qualification` example that requires root,
  `CAP_NET_ADMIN`, `CAP_NET_RAW`, and BPF privileges; creates a unique veth
  pair in the current network namespace; rejects pre-existing interface names;
  and records exact interface names, ifindexes, MAC addresses, and receive-drop
  counters.
- Disable IPv6 independently on both fixture interfaces when the per-interface
  sysctl exists, bring both links up, and pin the fixture process to one
  currently allowed CPU so the per-CPU aggregation key is deterministic.
- Open an `AF_PACKET/SOCK_RAW` socket without registering a receive protocol
  handler. Construct complete bounded Ethernet frames and transmit them through
  the peer interface with explicit destination/source MACs and EtherType.
- Load the production BPF object with enabled and required masks exactly
  `ATTACH_NETWORK_DROP`, EtherType exactly `0x88a4`, exact receive ifindex,
  threshold four, a bounded nonzero aggregation window, and no unrelated hook.
  Require a complete capability snapshot and Healthy initial heartbeat.
- Preserve `kfree_skb.reason` as the named `enum skb_drop_reason` BTF kind in
  the vendored tracepoint type and keep the observed value unsigned so Aya can
  apply the `FIELD_SIGNED` relocation against the target enum. Do not hard-code
  the runtime drop-reason value.
- Run negative controls before the formal injection: frames with a different
  EtherType on the forward link and EtherCAT frames in the reverse direction
  must produce no matching BPF network statistics, event, or incident.
- Require a zero formal baseline after controls for emitted/lost/network-drop,
  unattributed, and threshold-event statistics. Independent receive-drop
  counters may be nonzero in absolute terms, so compare bounded deltas.
- Observe one transport-risk cycle context before injection. Inject exactly
  four EtherCAT frames through the transmit peer and require the receive
  interface's independent `rx_dropped` delta to be at least four.
- Require exact final BPF statistics: `network_drops=4`,
  `network_unattributed=0`, `network_threshold_events=1`,
  `emitted_events=1`, and `lost_events=0`, with unrelated event counters zero.
- Require exactly one `KernelNetwork/NetworkDrop/Error` record with PID/TID
  zero, exact receive ifindex, observed value/threshold/count equal to four,
  positive bounded duration below the configured window, positive bounded
  kernel drop-reason detail, matching cycle identity, and nonzero evidence ID
  and timestamp.
- Require exactly one Error `HostNicDrop` incident with `ControlledStop`,
  confidence 75, matching evidence/ifindex/cycle identity, no dropped incident,
  and observer health changing from Healthy to Degraded with fault
  `0x45422001`.
- Bound link setup, controls, BPF polling, counter observation, and teardown.
  On any failure, detach the runtime, close the packet socket, delete the veth
  pair, and do not publish a qualified report.
- Remove stale final/temp reports at start and atomically publish
  `build/ebpf_network_drop_qualification.json` only after all assertions and
  interface cleanup pass. Add a closed-schema validator and regression tests.
- Add Make targets and a dedicated privileged GitHub Actions job. Build BPF and
  Rust without elevation, elevate only the final fixture, validate as the
  invoking user, and upload the validated report.
- Update README, software PRD, runtime observability design, capability
  manifest, and backend quality guidance with only the hosted virtual-veth
  unhandled-EtherType claim demonstrated by the report.

## Acceptance Criteria

- [ ] A privileged hosted Linux run verifies production CO-RE load and required
      `skb:kfree_skb` attachment with no unrelated hook.
- [ ] A unique veth pair is configured with exact interface identities and a
      process CPU affinity; protocol and direction negative controls remain
      silent before the formal run.
- [ ] Exactly four forward EtherCAT frames produce an independent receive
      `rx_dropped` delta of at least four and exact BPF drop/threshold/emission
      statistics with zero unattributed or lost records.
- [ ] Exactly one fixed network-drop record preserves the receive ifindex,
      threshold/count, kernel reason, aggregation duration, and transport-risk
      cycle identity.
- [ ] Exactly one Error `HostNicDrop` incident has `ControlledStop`, confidence
      75, matching evidence, and a Healthy-to-Degraded heartbeat transition
      with fault `0x45422001`.
- [ ] The report is same-directory atomic and its validator fails closed for
      schema, link identity/configuration, controls, attach masks, packet/drop
      counts, statistics, evidence, incident, loss, cleanup, and health
      mismatches.
- [ ] Focused Rust/Python tests, formatting, Clippy, BPF syntax, capability
      validation, shell/Python syntax, `make ci`, and `git diff --check` pass.
- [ ] The dedicated privileged GitHub Actions job passes and its downloaded
      artifact independently passes the repository validator.
- [ ] Documentation removes hosted virtual-veth unhandled-EtherType injection
      from the open list while preserving physical NIC/driver/NAPI/qdisc,
      queue-pressure, production-kernel, overhead/WCET, and long-duration HIL
      limits.

## Non-Goals

- Do not change host-wide network namespaces, routing, firewall, qdisc, sysctl,
  or physical interface configuration. Per-interface IPv6 suppression is
  limited to the disposable veth pair.
- Do not claim physical NIC, driver, NAPI, XDP, qdisc, queue-pressure,
  congestion, checksum, link failure, or real EtherCAT-device qualification.
- Do not hard-code a numeric kernel drop-reason enum across kernels; require a
  nonzero bounded detail and preserve it verbatim in evidence and incident.
- Do not change the fixed event/context/map ABI, evidence discriminants,
  attach bits, incident severity/action policy, MLG authority, or realtime
  path.
- Do not infer a production root cause from one virtual unhandled-protocol
  drop path or treat receive-interface counters as protocol-specific evidence.
- Missing `ip`, veth/raw-socket/BPF privilege, tracepoint, exact filtering,
  packet-drop evidence, cleanup success, or root/passwordless sudo must fail
  rather than skip qualification.

## Technical Notes

- Keep both veth ends in the current namespace. The receive path executes
  synchronously with the sender and avoids namespace handoff complexity.
- Use an `AF_PACKET` socket created with protocol zero and set the actual
  EtherType in `sockaddr_ll` and the Ethernet header for each send. This permits
  transmit without registering a receive handler for the injected protocol.
- Read ifindex, MAC, and `rx_dropped` from `/sys/class/net`; use the `ip` tool
  only for bounded fixture creation, link state, and deletion.
- Since the BPF map is keyed by CPU and ifindex, use the first CPU in the
  process's current affinity mask and require the emitted CPU to match it.
- Treat the exact BPF protocol/ifindex-filtered counters as authoritative for
  the formal packet count. The interface drop counter is independent lower-
  bound evidence and may include implementation-specific increments.
