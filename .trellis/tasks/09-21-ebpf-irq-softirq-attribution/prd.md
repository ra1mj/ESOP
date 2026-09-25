# eBPF IRQ and softirq attribution

## Goal

Complete the bounded Linux kernel-to-incident path for hard IRQ and softirq
duration evidence so ESOP can identify interrupt pressure that overlaps a
degraded EtherCAT cycle.

## Requirements

- Observe `irq_handler_entry`/`irq_handler_exit` and
  `softirq_entry`/`softirq_exit` tracepoint pairs.
- Track start times in fixed-capacity BPF maps keyed by CPU and interrupt
  vector. The maps must not grow without a declared bound.
- Emit a fixed 96-byte `RuntimeEvidence` record only when a configurable
  duration threshold is exceeded.
- Preserve the hard IRQ or softirq vector in the existing `u16` `irq` field,
  saturating out-of-range values instead of wrapping. Hard IRQ and softirq
  evidence must use distinct discriminants.
- Copy `boot_id`, `agent_epoch`, `cycle_seq`, and `transition_seq` from the
  existing kernel context map into every event.
- Keep IRQ observation independent of `tracked_pid`; interrupt handlers run in
  interrupt context and are correlated by CPU, cycle identity, and time window.
- Expose independent hard IRQ and softirq thresholds through `RuntimeConfig`,
  the kernel context map ABI, and a runtime update API.
- Require entry/exit attach points to be enabled and required as complete
  pairs. Invalid half-pair configurations must fail before loading BPF.
- Extend per-CPU statistics with samples and threshold overruns for each
  interrupt class, using saturating userspace aggregation.
- Correlate over-threshold hard IRQ and softirq evidence with an existing
  transport-risk cycle into `HOST_IRQ_STORM`. Evidence outside the configured
  correlation window must not be presented as a root cause.
- Keep new tracepoints optional in the default required mask so kernels that
  lack them can report reduced capability without breaking the existing
  required observation path.
- Update the PRD status, runtime observability design, capability manifest,
  README, and backend quality contract without claiming target-kernel
  qualification.

## Acceptance Criteria

- [x] The BPF bundle contains bounded hard IRQ and softirq entry/exit probes.
- [x] C and Rust context/stat structures have matching fixed sizes and fields.
- [x] The 96-byte evidence ABI decodes both IRQ evidence kinds without enum
      transmutation.
- [x] Runtime configuration rejects zero thresholds, unknown attach bits, and
      incomplete hard IRQ or softirq attach pairs.
- [x] Unit tests prove both evidence kinds produce `HOST_IRQ_STORM` only when
      correlated with a transport-risk cycle and over threshold.
- [x] Unit tests cover context preservation, statistics aggregation, attach
      mask completeness, and invalid configuration.
- [x] `make ci` passes locally.
- [x] GitHub Actions builds the real CO-RE BPF object successfully.

## Non-Goals

- Do not claim EBPF-003 target-kernel pressure-injection qualification from
  compile-time or unit-test evidence alone.
- Do not add a direct write path from eBPF to MLG, PDOs, controlwords, or motion
  permits.
- Do not implement windowed storm counting, CPU-overlap histograms, NIC IRQ
  identity mapping, or driver-specific probes in this increment.
- Do not make IRQ probes mandatory on every supported Linux kernel.

## Notes

- This increment advances FR-048 and the implementation prerequisites for
  EBPF-003. Real attach, verifier, pressure injection, and performance evidence
  remain target-environment work.
