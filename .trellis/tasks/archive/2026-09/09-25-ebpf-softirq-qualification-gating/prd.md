# Stabilize eBPF softirq qualification gating

## Goal

Eliminate hosted softirq qualification contamination without weakening the
existing one-record, one-incident, and exact-statistics qualification contract.
The qualifier must admit the selected CPU's `NET_RX` vector only while the
single controlled UDP GSO injection is being measured, then close the filter
before socket draining and ring-buffer polling.

## Background

GitHub Actions run `36154809386` failed at commit
`13a8802c2d39bfc5f1f379251f7d20cb83301204` with
`softirq injection produced records/incidents 2/2`. The immediately preceding
work commit passed the same job, and the final commit changed only Trellis
metadata. The qualifier currently leaves the selected CPU/vector filter open
from BPF load through final polling, so unrelated vector-3 work on that CPU can
be admitted after the empty baseline but outside the controlled `sendmsg`.

The existing `/proc/softirqs` contract already proves that the target CPU's
`NET_RX` counter advances exactly once around the GSO send. The BPF filter must
therefore cover that same bounded interval instead of the entire phase.

## Requirements

- Keep production IRQ/softirq observation semantics, kernel-context ABI,
  evidence ABI, attach masks, incident policy, and default all-CPU/all-vector
  behavior unchanged.
- Define one exact, impossible softirq vector value for the qualification's
  closed state. It must not reuse `u32::MAX`, because that value means all
  vectors in the production filter contract.
- Load each calibration/formal runtime with the selected CPU and the closed
  vector. Required attachments remain exactly the softirq entry/exit pair.
- After capability, agent, cycle, and zero-baseline checks, read the selected
  CPU's `NET_RX` counter, atomically open the runtime filter to vector 3, issue
  the one 64,800-byte UDP GSO send, read the counter again, and close the filter
  before receiving datagrams or polling the ring buffer.
- Attempt to close the filter on every post-open error path. A failed close,
  partial send, malformed `/proc/softirqs` sample, CPU migration, or counter
  delta other than one is a hard failure with no report.
- Verify the runtime's userspace kernel-context mirror after the initial closed
  configuration, active vector-3 update, and final closed update in both
  calibration and formal phases.
- Preserve exact phase assertions: one ring-buffer record, one correlated
  incident, one sample, one overrun, one emitted event, zero hard-IRQ activity,
  zero loss, and no malformed, rejected, or dropped evidence.
- Extend the atomic JSON report with the closed vector and calibration/formal
  before/active/after vector snapshots. The strict validator must require the
  closed-target-closed sequence and reject unknown, missing, mistyped, or
  inconsistent gate fields.
- Add regression mutations for the new gate contract while retaining all
  existing schema, injection, evidence, incident, and health checks.
- Update README, software PRD, observability design, capability manifest, and
  backend quality guidance to describe temporal CPU/vector gating and the
  hosted-noise boundary accurately.
- Run focused tests and the complete `make ci` gate, commit and push to
  `ra1mj/ESOP`, then require the final GitHub Actions run and dedicated
  `ebpf-softirq-runtime` job to pass. Downloaded evidence must independently
  pass the repository validator.

## Acceptance Criteria

- [x] Both runtime phases start with the selected CPU plus the closed vector,
      expose vector 3 only across the controlled send/counter interval, and
      return to the closed vector before receive/poll work.
- [x] Every opened gate is closed even when the send or post-send counter read
      fails; no retry or relaxed count contract is introduced.
- [x] The qualifier still requires exactly one target-CPU `NET_RX` delta,
      record, incident, sample, overrun, and emission with zero loss.
- [x] The report validator proves the closed/open/closed sequence for both
      calibration and formal phases and rejects gate-field mutations.
- [x] Existing eBPF runtime/unit/report tests, formatting, Clippy, BPF syntax,
      capability validation, shell/Python syntax, and `make ci` pass locally.
- [x] The final pushed commit's complete GitHub Actions workflow succeeds; the
      softirq artifact is downloaded, checksum-accounted, and independently
      accepted by `validate-ebpf-softirq-qualification.py`.
- [x] Durable quality guidance records that exact CPU/vector filtering is not
      temporal isolation unless the qualification closes the filter outside
      the controlled injection interval.

## Notes

- This task repairs qualification isolation only. It does not claim hard IRQ,
  physical NIC/driver/NAPI behavior, production thresholds, sustained pressure,
  task causality, production kernels, overhead/WCET, or long-duration HIL.
