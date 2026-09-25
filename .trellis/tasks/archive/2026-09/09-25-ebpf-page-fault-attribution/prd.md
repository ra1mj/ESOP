# eBPF page fault window attribution

## Goal

Complete the bounded Linux kernel-to-incident path for tracked-process user
page faults so ESOP can identify a page-fault burst that overlaps a degraded
cycle without emitting one ring-buffer record per fault.

## Background

The current `exceptions:page_fault_user` program emits one event for every
matching fault and the userspace correlator treats any correlated event as
`HOST_PAGE_FAULT`. Linux exposes the fault address, instruction pointer, and
architecture-specific error code at this tracepoint, but it does not expose
the eventual major/minor result or fault-handling duration. FR-048, FR-050,
and EBPF-005 therefore require a bounded count-window signal rather than an
unbounded event stream or an unsupported duration claim.

## Requirements

- Read `page_fault_user` through its typed tracepoint context and retain the
  architecture-specific error code as saturated kind-specific evidence
  detail.
- Restrict aggregation to `tracked_pid` when it is nonzero. Production
  deployments should set this to the EtherCAT/gateway process; zero remains a
  diagnostic all-process mode.
- Aggregate faults in a fixed-capacity LRU BPF map keyed by CPU and process.
  Each entry must have an explicit monotonic time window and saturating count.
- Emit at most one fixed 96-byte evidence record when a window first reaches
  the configured count threshold. Preserve the triggering PID/TID, CPU,
  count, threshold, elapsed window, boot/agent/cycle identity, and error-code
  detail.
- Expose page-fault count threshold and window duration through
  `RuntimeConfig`, the C/Rust kernel context ABI, and an atomic runtime update
  method. Reject zero values and thresholds above `u32::MAX`.
- Extend per-CPU statistics with emitted page-fault threshold windows while
  retaining total observed page-fault count and saturating userspace
  aggregation.
- Correlate page-fault evidence into `HOST_PAGE_FAULT` only when the evidence
  count reaches its threshold and overlaps an existing deadline/WKC/DC-risk
  cycle. Below-threshold or healthy-cycle evidence must not be presented as a
  root cause.
- Keep the page-fault tracepoint optional in the default required attach mask
  so kernels without that tracepoint report reduced capability without
  breaking the existing scheduler/process baseline.
- Update the software PRD, runtime observability design, README, and backend
  quality contract without claiming major/minor classification, target-kernel
  pressure injection, or production verifier qualification.

## Acceptance Criteria

- [x] The BPF bundle contains a 256-entry LRU page-fault window map keyed by
      CPU and process.
- [x] A window emits only at the first threshold crossing and resets after its
      configured duration.
- [x] The fixed 96-byte evidence ABI decodes triggering PID/TID, count,
      threshold, elapsed window, and error-code detail without enum
      transmutation.
- [x] Runtime configuration and update helpers reject zero/oversized page-
      fault policies without partially mutating the kernel context.
- [x] Unit tests prove `HOST_PAGE_FAULT` requires both threshold evidence and
      a correlated cycle-risk window.
- [x] Unit tests cover context preservation, C/Rust map ABI sizes, and
      saturating page-fault statistics.
- [x] `make ci` passes locally.
- [x] GitHub Actions builds the real CO-RE BPF object successfully.

## Non-Goals

- Do not infer major versus minor faults from the architecture error code;
  that outcome is not present in this tracepoint record.
- Do not report fault-handler latency; tracepoint timestamps delimit observed
  fault arrivals, not handling completion.
- Do not add full fault address or instruction-pointer fields by repurposing
  IRQ/network fields in the stable evidence record.
- Do not claim EBPF-005 target-kernel memory-pressure/OOM/page-fault injection
  qualification from compile-time or unit-test evidence alone.
- Do not let eBPF evidence write MLG state, PDOs, CiA 402 controlwords, or
  motion permits.

## Notes

- Linux defines the raw event class with `address`, `ip`, and `error_code`.
  The stable ESOP record intentionally keeps count semantics in
  `observed_value` and `count`; full address attribution needs a future ABI or
  separate diagnostic channel.
