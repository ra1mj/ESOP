# Result: eBPF network-drop runtime qualification

Date: 2026-09-25

## Delivered

- Added a Linux-only qualification fixture that creates an isolated veth pair,
  disables per-link IPv6, pins itself to one inherited CPU, and injects complete
  protocol-zero AF_PACKET Ethernet frames without registering a receive handler.
- Loaded the production CO-RE object with only `ATTACH_NETWORK_DROP` enabled and
  required, qualified protocol and receive-ifindex controls, and proved the
  four-drop threshold through independent link counters and exact BPF statistics.
- Verified one fixed `KernelNetwork/NetworkDrop/Error` evidence item, one Error
  `HostNicDrop`/`ControlledStop` incident at confidence 75, and a
  Healthy-to-Degraded observer-health transition.
- Added a closed-schema validator, six regression tests, Make targets, a
  dedicated privileged GitHub Actions job, artifact upload, capability evidence,
  PRD/design/README text, and backend quality contracts.
- Corrected the vendored `kfree_skb.reason` BTF model to preserve the target
  `enum skb_drop_reason` kind and added compile-time kind/width assertions. This
  prevents Aya from poisoning an impossible `FIELD_SIGNED` relocation as helper
  `0x0bad2310` during target-kernel load.
- Preserved the narrow claim: this qualifies one hosted virtual-veth
  unhandled-EtherType receive-drop chain, not physical NIC, driver, NAPI, XDP,
  qdisc, queue pressure, congestion, real EtherCAT device, production-kernel,
  overhead, WCET, or long-duration HIL behavior.

## Verification

- Work commits: `df732696e3e99343c82f1b30b45a11594803fc0c`,
  `4a45b9f92621f28f891361cd737f62d756d47809`, and
  `5a5294b684641961db1397cfa10989d508d3a563`.
- GitHub Actions run: `36142735199`.
- Run URL: `https://github.com/ra1mj/ESOP/actions/runs/36142735199`.
- All 11 jobs passed, including `ebpf-network-drop-runtime`, all existing
  privileged eBPF qualifications, CO-RE object build, and the full Rust/Zenoh
  quality job.
- Downloaded artifact:
  `esop-ebpf-network-drop-runtime-qualification/ebpf_network_drop_qualification.json`.
- The downloaded report independently passed
  `scripts/validate-ebpf-network-drop-qualification.py` and has SHA-256
  `d54382f7b70805734a662ab68b9438d92c713b4c951784e2cae17c786c503fe0`.
- Local `make ci`, focused Rust tests, six validator tests, BPF syntax, real
  Clang BPF object generation/BTF inspection, capability-manifest validation,
  formatting, Clippy, release/no-std builds, and `git diff --check` passed.

## Hosted measurements

- Interfaces `esotx1f4e`/`esorx1f4e` used ifindexes 6/5, distinct unicast MACs,
  MTU 1500, link-up state, and per-link IPv6 disablement. The fixture was pinned
  to CPU 0 and used packet-socket protocol zero.
- Controls sent one forward `0x88a5` frame and one reverse `0x88a4` frame. Each
  expected independent receive-drop counter advanced once while matching BPF
  counters, records, incidents, loss, and health remained zero/healthy.
- Four 64-byte forward EtherCAT frames changed RX drops from 1 to 5, for an
  exact formal delta of 4. TX drops stayed at 1, for an exact formal delta of 0.
- Attach masks were exactly 32 (`ATTACH_NETWORK_DROP`), threshold 4, and window
  1,000,000,000 ns.
- BPF/poll results were `network_drops=4`, `network_unattributed=0`, one threshold
  event, one emitted/seen record, one incident, zero loss, zero malformed or
  rejected evidence, and zero dropped incidents.
- Evidence used CPU 0, RX ifindex 5, detail 74, duration 13,756 ns, value/
  threshold/count 4, cycle 42, and transition 9.
- The incident was Error `HostNicDrop`, action `ControlledStop`, confidence 75.
  Observation changed from Healthy heartbeat 1 to Degraded heartbeat 2 with
  fault `0x45422001`.
- Runtime detach, packet-socket close, link deletion, cleanup verification, and
  affinity restoration all succeeded before the qualified report was published.

## Debug retrospective

- Category: implicit cross-layer BTF assumption plus a target-load test gap.
- Initial source/object checks did not exercise Aya's relocation against target
  kernel BTF. The first hosted run exposed a poisoned field relocation. Changing
  signedness alone left local `INT` versus target `ENUM` incompatible; preserving
  the named enum kind resolved the relocation.
- Prevention now combines compile-time kind/width assertions, object BTF
  inspection, the backend quality contract, and the mandatory privileged
  target-kernel verifier/load job.

## Residual limits

The result does not establish physical-NIC or driver drop behavior, NAPI/XDP/
qdisc attribution, queue pressure or congestion cause, checksum/link failure,
real EtherCAT-device behavior, production target-kernel portability, overhead,
WCET, sustained load, or long-duration HIL stability.
