# Journal - mj (Part 1)

> AI development session journal
> Started: 2026-09-03

---


## Session 1: Complete CiA 402 cyclic PDO binding

**Date**: 2026-09-09
**Task**: Complete CiA 402 cyclic PDO binding
**Branch**: `main`

### Summary

Resumed the CiA 402 cyclic PDO task, verified the fixed-layout adapter and SII projection binding, ran make ci successfully, and archived the completed Trellis task.

### Main Changes

- Detailed change bullets were not supplied; see the summary above.

### Git Commits

| Hash | Message |
|------|---------|
| `52a6502` | (see git log) |
| `61f4d86` | (see git log) |
| `4116c96` | (see git log) |

### Testing

- Validation was not recorded for this session.

### Status

[OK] **Completed**

### Next Steps

- None - task complete


## Session 2: R2 qualification evidence gate

**Date**: 2026-09-21
**Task**: R2 qualification evidence gate
**Branch**: `main`

### Summary

Added a fail-closed R2 qualification manifest, HIL topology and limitations artifacts, strict evidence/hash/coherence validation, CI reporting, regression tests, and documentation. Local CI, simulator HIL, Zenoh integration, and GitHub Actions run 35578603863 passed; checked-in product baseline remains explicitly unqualified pending real hardware evidence.

### Main Changes

- Detailed change bullets were not supplied; see the summary above.

### Git Commits

| Hash | Message |
|------|---------|
| `9517f9d` | (see git log) |

### Testing

- Validation was not recorded for this session.

### Status

[OK] **Completed**

### Next Steps

- None - task complete


## Session 3: eBPF IRQ and softirq attribution

**Date**: 2026-09-25
**Task**: eBPF IRQ and softirq attribution
**Branch**: `main`

### Summary

Added bounded hard IRQ and softirq duration probes, transactional attach-pair handling, runtime thresholds and statistics, HOST_IRQ_STORM correlation, tests, documentation, and successful local plus GitHub CO-RE validation.

### Main Changes

- Detailed change bullets were not supplied; see the summary above.

### Git Commits

| Hash | Message |
|------|---------|
| `dc11d28` | (see git log) |
| `59b271c` | (see git log) |

### Testing

- Validation was not recorded for this session.

### Status

[OK] **Completed**

### Next Steps

- None - task complete


## Session 4: eBPF EtherCAT drop attribution

**Date**: 2026-09-25
**Task**: eBPF EtherCAT drop attribution
**Branch**: `main`

### Summary

Added bounded per-CPU/per-interface EtherCAT skb drop windows, typed CO-RE ifindex/drop-reason attribution, atomic runtime policy updates, network statistics, HOST_NIC_DROP cycle-risk correlation, tests, and PRD/capability documentation; local and GitHub quality gates passed.

### Main Changes

- Detailed change bullets were not supplied; see the summary above.

### Git Commits

| Hash | Message |
|------|---------|
| `8690b77` | (see git log) |

### Testing

- Validation was not recorded for this session.

### Status

[OK] **Completed**

### Next Steps

- None - task complete


## Session 5: eBPF page fault window attribution

**Date**: 2026-09-25
**Task**: eBPF page fault window attribution
**Branch**: `main`

### Summary

Added a bounded 256-entry CPU/process page-fault window, threshold-only fixed evidence with error-code detail, atomic runtime policy updates and statistics, HOST_PAGE_FAULT cycle-risk correlation, tests, capability/PRD documentation, and successful local plus GitHub CO-RE/Rust/Zenoh validation (run 36092132507).

### Main Changes

- Detailed change bullets were not supplied; see the summary above.

### Git Commits

| Hash | Message |
|------|---------|
| `1394f6f` | (see git log) |

### Testing

- Validation was not recorded for this session.

### Status

[OK] **Completed**

### Next Steps

- None - task complete


## Session 6: eBPF process and OOM attribution

**Date**: 2026-09-25
**Task**: eBPF process and OOM attribution
**Branch**: `main`

### Summary

Prevented tracked worker-thread exits from becoming critical component-exit incidents, attributed OOM hard facts to mark_victim PID, expanded saturating per-CPU statistics, updated lifecycle observability contracts, and verified local plus GitHub CO-RE/Rust/Zenoh quality gates.

### Main Changes

- Detailed change bullets were not supplied; see the summary above.

### Git Commits

| Hash | Message |
|------|---------|
| `703be95` | (see git log) |

### Testing

- Validation was not recorded for this session.

### Status

[OK] **Completed**

### Next Steps

- None - task complete


## Session 7: eBPF CPU frequency limit attribution

**Date**: 2026-09-25
**Task**: eBPF CPU frequency limit attribution
**Branch**: `main`

### Summary

Added bounded cpu_frequency_limits tracing, runtime floor and policy filtering, episode suppression and recovery, saturating statistics, correlated HOST_CPU_THROTTLE classification, ABI tests, documentation, capability evidence, and real CO-RE CI validation.

### Main Changes

- Detailed change bullets were not supplied; see the summary above.

### Git Commits

| Hash | Message |
|------|---------|
| `e935969` | (see git log) |

### Testing

- Validation was not recorded for this session.

### Status

[OK] **Completed**

### Next Steps

- None - task complete


## Session 8: eBPF CPU migration attribution

**Date**: 2026-09-25
**Task**: eBPF CPU migration attribution
**Branch**: `main`

### Summary

Implemented bounded sched_migrate_task observation with per-TID epoch-aware migration windows, fixed evidence decoding, correlated lower-confidence scheduler incidents, ABI and stats updates, documentation, local quality gates, and passing GitHub Rust/CO-RE CI.

### Main Changes

- Detailed change bullets were not supplied; see the summary above.

### Git Commits

| Hash | Message |
|------|---------|
| `5065098` | (see git log) |

### Testing

- Validation was not recorded for this session.

### Status

[OK] **Completed**

### Next Steps

- None - task complete


## Session 9: eBPF gateway stall attribution

**Date**: 2026-09-25
**Task**: eBPF gateway stall attribution
**Branch**: `main`

### Summary

Added stable Zenoh publish lifecycle markers, transactional gateway uprobes, bounded eBPF stall evidence, correlated lifecycle classification, tests, and truthful observability documentation; local and GitHub quality gates passed.

### Main Changes

- Detailed change bullets were not supplied; see the summary above.

### Git Commits

| Hash | Message |
|------|---------|
| `0d163fa` | (see git log) |

### Testing

- Validation was not recorded for this session.

### Status

[OK] **Completed**

### Next Steps

- None - task complete


## Session 10: eBPF gateway callback stall attribution

**Date**: 2026-09-25
**Task**: eBPF gateway callback stall attribution
**Branch**: `main`

### Summary

Added stable Zenoh command/query callback markers, shared gateway request IDs, BPF operation-class validation, atomic Aya callback probe attachment, fixed-ABI coverage, live Zenoh validation, and truthful observability documentation.

### Main Changes

- Detailed change bullets were not supplied; see the summary above.

### Git Commits

| Hash | Message |
|------|---------|
| `5ab9ee1` | (see git log) |

### Testing

- Validation was not recorded for this session.

### Status

[OK] **Completed**

### Next Steps

- None - task complete


## Session 11: eBPF Linux raw-port stall attribution

**Date**: 2026-09-25
**Task**: eBPF Linux raw-port stall attribution
**Branch**: `main`

### Summary

Added stable Linux raw-port syscall markers, bounded eBPF duration tracking, transactional runtime attachment, HOST_PORT_STALL correlation, ABI tests, documentation, and qualification checks; local and GitHub quality gates passed.

### Main Changes

- Detailed change bullets were not supplied; see the summary above.

### Git Commits

| Hash | Message |
|------|---------|
| `45e633b` | (see git log) |

### Testing

- Validation was not recorded for this session.

### Status

[OK] **Completed**

### Next Steps

- None - task complete


## Session 12: Qualify eBPF raw-port runtime path

**Date**: 2026-09-25
**Task**: Qualify eBPF raw-port runtime path
**Branch**: `main`

### Summary

Added a privileged Linux qualification that verifies the real CO-RE object, exact raw-port uprobe pair, delayed Unix recv evidence, ring-buffer decoding, kernel statistics, HostPortStall correlation, exact-schema report validation, dedicated GitHub Actions evidence, and explicit AF_PACKET/hardware/WCET limits. Local full CI and remote quality run 36108463279 passed.

### Main Changes

- Detailed change bullets were not supplied; see the summary above.

### Git Commits

| Hash | Message |
|------|---------|
| `3963e26` | (see git log) |

### Testing

- Validation was not recorded for this session.

### Status

[OK] **Completed**

### Next Steps

- None - task complete


## Session 13: Privileged eBPF gateway runtime qualification

**Date**: 2026-09-25
**Task**: Privileged eBPF gateway runtime qualification
**Branch**: `main`

### Summary

Implemented and documented the privileged Zenoh gateway eBPF marker-to-incident qualification. Added the production-symbol fixture, exact-schema validator and regression tests, minimized-privilege runner, Make/CI integration, and bounded claims. Local quality gates passed; GitHub Actions run 36111379291 passed all jobs and its uploaded report was downloaded and revalidated with two over-threshold gateway records, one merged incident, full attach mask, and zero mismatch/loss.

### Main Changes

- Detailed change bullets were not supplied; see the summary above.

### Git Commits

| Hash | Message |
|------|---------|
| `2fc62b8` | (see git log) |

### Testing

- Validation was not recorded for this session.

### Status

[OK] **Completed**

### Next Steps

- None - task complete


## Session 14: eBPF process-exit runtime qualification

**Date**: 2026-09-25
**Task**: eBPF process-exit runtime qualification
**Branch**: `main`

### Summary

Added and qualified the hosted Linux sched_process_exit path with synchronized worker suppression, leader hard-fact incident and failed observer checks, exact-schema validation, CI artifact evidence, and bounded documentation claims.

### Main Changes

- Detailed change bullets were not supplied; see the summary above.

### Git Commits

| Hash | Message |
|------|---------|
| `309a835` | (see git log) |

### Testing

- Validation was not recorded for this session.

### Status

[OK] **Completed**

### Next Steps

- None - task complete


## Session 15: Qualify eBPF scheduler migration runtime

**Date**: 2026-09-25
**Task**: Qualify eBPF scheduler migration runtime
**Branch**: `main`

### Summary

Added an exact-TID two-CPU scheduler migration qualification fixture, strict report validator and tests, privileged GitHub Actions coverage, and bounded documentation claims. Local make ci and GitHub Actions run 36117944042 passed; the downloaded scheduler artifact independently validated with exact counts and zero loss.

### Main Changes

- Detailed change bullets were not supplied; see the summary above.

### Git Commits

| Hash | Message |
|------|---------|
| `f1aeddc` | (see git log) |

### Testing

- Validation was not recorded for this session.

### Status

[OK] **Completed**

### Next Steps

- None - task complete


## Session 16: Qualify eBPF scheduler runqueue latency

**Date**: 2026-09-25
**Task**: Qualify eBPF scheduler runqueue latency
**Branch**: `main`

### Summary

Added privileged hosted-Linux exact-TID scheduler runqueue latency qualification, strict report validation and CI artifact coverage; corrected fallback evidence-ID semantics and stabilized scheduler-migration setup with an inert pre-worker TID. Local make ci and GitHub Actions run 36123158759 passed, and downloaded runqueue/migration artifacts passed independent validation.

### Main Changes

- Detailed change bullets were not supplied; see the summary above.

### Git Commits

| Hash | Message |
|------|---------|
| `7b89813` | (see git log) |
| `bc9891a` | (see git log) |
| `adc6131` | (see git log) |

### Testing

- Validation was not recorded for this session.

### Status

[OK] **Completed**

### Next Steps

- None - task complete


## Session 17: Qualify hosted eBPF softirq duration

**Date**: 2026-09-25
**Task**: Qualify hosted eBPF softirq duration
**Branch**: `main`

### Summary

Added CPU/vector-filtered IRQ and softirq tracking, a controlled loopback NET_RX calibration/formal qualification, strict report validation, CI artifact proof, and documentation/spec updates. Local make ci and GitHub Actions run 36127109470 passed; the downloaded report independently validated one sample/event/incident with zero loss.

### Main Changes

- Detailed change bullets were not supplied; see the summary above.

### Git Commits

| Hash | Message |
|------|---------|
| `73d05e7` | (see git log) |
| `275a816` | (see git log) |

### Testing

- Validation was not recorded for this session.

### Status

[OK] **Completed**

### Next Steps

- None - task complete


## Session 18: eBPF page-fault runtime qualification

**Date**: 2026-09-25
**Task**: eBPF page-fault runtime qualification
**Branch**: `main`

### Summary

Added and qualified the controlled hosted x86_64 page-fault count-window chain, strict report validation, CI artifact verification, capability/docs updates, and preserved semantic and production limits.

### Main Changes

- Detailed change bullets were not supplied; see the summary above.

### Git Commits

| Hash | Message |
|------|---------|
| `ba26760e7750a4cd149b3e9021a0007d5eef470c` | (see git log) |

### Testing

- Validation was not recorded for this session.

### Status

[OK] **Completed**

### Next Steps

- None - task complete


## Session 19: Qualify hosted memcg OOM eBPF path

**Date**: 2026-09-25
**Task**: Qualify hosted memcg OOM eBPF path
**Branch**: `main`

### Summary

Added a fail-closed cgroup-v2 OOM runtime fixture, strict report validator/tests, dedicated privileged CI artifact, and narrow FR-048/EBPF-005 documentation. GitHub Actions run 36134613286 passed all 10 jobs; the downloaded report independently validated one local OOM kill, one BPF incident, zero loss, Failed observation, and successful cleanup.

### Main Changes

- Detailed change bullets were not supplied; see the summary above.

### Git Commits

| Hash | Message |
|------|---------|
| `560e85d` | (see git log) |

### Testing

- Validation was not recorded for this session.

### Status

[OK] **Completed**

### Next Steps

- None - task complete


## Session 20: Qualify hosted eBPF network-drop path

**Date**: 2026-09-25
**Task**: Qualify hosted eBPF network-drop path
**Branch**: `main`

### Summary

Added an isolated veth/AF_PACKET network-drop runtime fixture, strict validator/tests, dedicated privileged CI artifact, FR-048/EBPF-004 documentation, and BTF enum-kind compile-time protection. GitHub Actions run 36142735199 passed all 11 jobs; the downloaded report independently validated four filtered drops, one HostNicDrop incident, zero loss, Degraded observation, and complete cleanup.

### Main Changes

- Detailed change bullets were not supplied; see the summary above.

### Git Commits

| Hash | Message |
|------|---------|
| `df73269` | (see git log) |
| `4a45b9f` | (see git log) |
| `5a5294b` | (see git log) |

### Testing

- Validation was not recorded for this session.

### Status

[OK] **Completed**

### Next Steps

- None - task complete


## Session 21: Qualify eBPF observability degradation recovery

**Date**: 2026-09-26
**Task**: Qualify eBPF observability degradation recovery
**Branch**: `main`

### Summary

Added sticky epoch-local eBPF event-loss health, restart reset semantics, a production 4 MiB ringbuf saturation/unload-reload qualification, strict validator/tests, CI artifact, and synchronized PRD/spec/capability evidence. GitHub Actions run 36151603726 passed all 12 jobs; the downloaded artifact independently validated 40,329 emitted events, 8,823 exact projected losses, epoch-2 Healthy recovery, zero recovery loss/rejection, and complete cleanup.

### Main Changes

- Detailed change bullets were not supplied; see the summary above.

### Git Commits

| Hash | Message |
|------|---------|
| `9db984178f79157d177d2688e665b902d434d204` | (see git log) |

### Testing

- Validation was not recorded for this session.

### Status

[OK] **Completed**

### Next Steps

- None - task complete


## Session 22: Stabilize eBPF softirq qualification gating

**Date**: 2026-09-26
**Task**: Stabilize eBPF softirq qualification gating
**Branch**: `main`

### Summary

Bounded hosted softirq observation to the exact GSO injection interval, preserved strict one-event evidence, passed all local quality gates and GitHub Actions run 36160745503, independently validated artifact 10876025194, and archived the completed task.

### Main Changes

- Detailed change bullets were not supplied; see the summary above.

### Git Commits

| Hash | Message |
|------|---------|
| `d2b7eeb9641fec87444f2aabc2ae6ba7855ae414` | (see git log) |

### Testing

- Validation was not recorded for this session.

### Status

[OK] **Completed**

### Next Steps

- None - task complete


## Session 23: Runtime incident Protobuf and Zenoh projection

**Date**: 2026-09-26
**Task**: Runtime incident Protobuf and Zenoh projection
**Branch**: `main`

### Summary

Added lossless eBPF agent incident projection into additive Protobuf v1 fields, strict query validation, production Zenoh publication, frozen-reader compatibility, and live router coverage.

### Main Changes

- Detailed change bullets were not supplied; see the summary above.

### Git Commits

| Hash | Message |
|------|---------|
| `31dfb89` | (see git log) |

### Testing

- Validation was not recorded for this session.

### Status

[OK] **Completed**

### Next Steps

- None - task complete


## Session 24: Implement bounded Unix domain IPC transport

**Date**: 2026-09-26
**Task**: Implement bounded Unix domain IPC transport
**Branch**: `main`

### Summary

Added the host-only esop-ipc crate with a fixed 80-byte little-endian frame, bounded payload CRC, nonblocking Unix datagram endpoint, peer identity/replay/time/restart/offline monitoring, kernel-backed integration tests, FR-029/R3 documentation, capability evidence, and cross-layer quality guidance. Local make ci and GitHub Actions quality run 36171647431 passed.

### Main Changes

- Detailed change bullets were not supplied; see the summary above.

### Git Commits

| Hash | Message |
|------|---------|
| `a8cd6f7` | (see git log) |

### Testing

- Validation was not recorded for this session.

### Status

[OK] **Completed**

### Next Steps

- None - task complete


## Session 25: Connect ProcBuf payloads to hosted IPC

**Date**: 2026-09-26
**Task**: Connect ProcBuf payloads to hosted IPC
**Branch**: `main`

### Summary

Added the shared ProcBuf/Protobuf IPC payload adapter, delegated Zenoh projection and command decoding to it, enforced envelope/payload identity before CommandIngress, and covered state/event/command flows over real Unix datagrams.

### Main Changes

- Detailed change bullets were not supplied; see the summary above.

### Git Commits

| Hash | Message |
|------|---------|
| `4ee4b79` | (see git log) |

### Testing

- Validation was not recorded for this session.

### Status

[OK] **Completed**

### Next Steps

- None - task complete


## Session 26: Complete command target ProcBuf handoff

**Date**: 2026-09-26
**Task**: Complete command target ProcBuf handoff
**Branch**: `main`

### Summary

Published validated CSP/CSV/CST command targets into ProcBuf ABI v5 with exact lifecycle permit identity, shared IPC/Zenoh admission adapters, end-to-end Unix datagram coverage, updated capability documentation, and passing local plus GitHub quality gates.

### Main Changes

- Detailed change bullets were not supplied; see the summary above.

### Git Commits

| Hash | Message |
|------|---------|
| `7d2029168119c499c98dd24c4996ae37df29e0bd` | (see git log) |

### Testing

- Validation was not recorded for this session.

### Status

[OK] **Completed**

### Next Steps

- None - task complete


## Session 27: ProcBuf CiA 402 target execution

**Date**: 2026-09-26
**Task**: ProcBuf CiA 402 target execution
**Branch**: `main`

### Summary

Implemented fail-closed ProcBuf command validation and deterministic CiA 402 PDO target execution, exposed direct and scheduled stop-cycle APIs, added Linux E2E coverage, updated qualification evidence and PRD documentation, and verified the complete local and GitHub Actions quality gates.

### Main Changes

- Detailed change bullets were not supplied; see the summary above.

### Git Commits

| Hash | Message |
|------|---------|
| `01c6626` | (see git log) |

### Testing

- Validation was not recorded for this session.

### Status

[OK] **Completed**

### Next Steps

- None - task complete


## Session 28: Publish CiA 402 feedback into ProcBuf state

**Date**: 2026-09-26
**Task**: Publish CiA 402 feedback into ProcBuf state
**Branch**: `main`

### Summary

Published verified CiA 402 feedback and transport-accepted controlwords through ProcBuf ABI v6 and Protobuf, with transactional stale-safe projection, Linux end-to-end coverage, updated product contracts, and passing local plus GitHub quality gates.

### Main Changes

### Verification

- `make ci` passed.
- `make test-zenoh` passed both live integration tests.
- `make bpf CLANG="$HOME/.local/opt/clang14/usr/bin/clang-14"` passed after installing Clang 14 under the user-local prefix.
- GitHub Actions quality run `36189150904` passed every Rust, BPF/eBPF privileged runtime, and Zenoh job.

### Remaining Qualification Scope

- Physical EtherCAT HIL and servo tuning remain pending.
- Product-specific process-image mapping and product-scale axis configuration remain pending.
- Certified WCET measurement remains pending.


### Git Commits

| Hash | Message |
|------|---------|
| `1411fb26aa4521169904a240bf44502f80fa3931` | (see git log) |

### Testing

- Validation was not recorded for this session.

### Status

[OK] **Completed**

### Next Steps

- None - task complete
