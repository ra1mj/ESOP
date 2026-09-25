# eBPF softirq runtime qualification

## Goal

Qualify the existing FR-048/EBPF-003 softirq-duration observation path on a
privileged hosted Linux kernel. The evidence must prove that the production
CO-RE object can load and require the softirq tracepoint pair, isolate one
controlled loopback `NET_RX` burst on an allowed CPU, measure its handler
duration, correlate it with a transport-risk cycle, and project the resulting
`HOST_IRQ_STORM` incident into observer health.

## Background

The repository already implements bounded hard-IRQ and softirq entry/exit
tracking, fixed 96-byte duration evidence, per-CPU statistics, pairwise attach
rollback, and cycle-risk correlation. Source tests and CO-RE compilation do not
establish real verifier/load behavior, real softirq execution, isolated
CPU/vector attribution, ringbuf delivery, or end-to-end health projection.

Linux traces each softirq vector around the complete vector action. Loopback
transmit queues receive work through `__netif_rx`; a single UDP GSO send can
therefore inject a bounded `NET_RX` burst without external network hardware.
Local unprivileged probes showed one target-CPU `NET_RX` execution for 8-54
`UDP_SEGMENT` datagrams, while `sendmmsg` generated one execution per message.

## Requirements

- Add explicit interrupt CPU/vector filters to `RuntimeConfig` and the existing
  kernel context ABI by renaming the current reserved `u16`/`u32` fields. The
  all-CPU/all-vector defaults must preserve current behavior and structure size.
- Apply the filters before inserting hard-IRQ or softirq start timestamps.
  Filtering must remain independent from `tracked_pid` and must not create
  half-pair state.
- Add an atomic runtime update API for both filter values. Unit tests must cover
  defaults, cycle-context preservation, updates, ABI size, and BPF syntax.
- Keep separate `HOST_IRQ_STORM` incidents for hard IRQ versus softirq and for
  different CPU/vector identities. Same-identity evidence may still merge in
  the existing bounded correlation window.
- Add a Linux-only `softirq_qualification` example that selects a quiet CPU
  from the process's actual allowed affinity set, pins itself to that CPU, and
  requires CPU/vector IDs representable by the evidence ABI.
- Load the production object with enabled and required masks both exactly equal
  to `ATTACH_SOFTIRQ_ENTRY | ATTACH_SOFTIRQ_EXIT`; unrelated programs stay
  disabled. Configure the interrupt filter to the selected CPU and Linux
  `NET_RX` vector 3.
- Use a connected IPv4 loopback UDP socket and one `UDP_SEGMENT` `sendmsg` with
  a fixed 54-datagram burst. Require the complete payload to be sent and all 54
  datagrams to be received within bounded deadlines.
- Perform a calibration load with a one-nanosecond threshold, require exactly
  one matching record with zero loss, then unload it. Derive the formal
  threshold as `max(1, calibration_duration_ns / 8)` and run the same injection
  through a fresh runtime and fresh `RuntimeAgent`.
- Publish the same deadline/WKC-risk `CycleContext` to the formal runtime and
  agent before injection. The formal run must start Healthy and become Degraded
  with fault `0x45422001` after correlation.
- Require one `KernelIrq/SoftirqCpuTime/Error` record on the selected CPU with
  vector 3, the exact fixture PID/TID, zero ifindex/detail, count one,
  `observed_value == duration_ns > threshold`, the injected cycle identity,
  and a nonzero evidence ID/timestamp.
- Require one Error `HostIrqStorm` incident with `ControlledStop`, confidence
  70, matching CPU/vector/cycle/timing fields, one retained evidence item, and
  no dropped incident.
- Require exact formal statistics for the filtered path:
  `softirq_samples=1`, `softirq_overruns=1`, `emitted_events=1`, and zero loss;
  hard-IRQ samples/overruns must remain zero. Polling must report one record,
  one incident, and zero malformed, rejected, or newly reported lost records.
- Remove stale final/temp reports at start and atomically publish
  `build/ebpf_softirq_qualification.json` only after every assertion passes.
  Add a strict allowlist validator and regression tests for schema and semantic
  failures.
- Add Make targets and a dedicated privileged GitHub Actions job. Build BPF and
  Rust without elevation, elevate only the final fixture, validate as the
  invoking user, and upload the validated report.
- Update README, software PRD, runtime observability design, capability
  manifest, and backend quality guidance to claim only the controlled hosted
  loopback `NET_RX` softirq chain demonstrated by the report.

## Acceptance Criteria

- [ ] A privileged hosted Linux run verifies production CO-RE load, required
      softirq entry/exit attachment, exact CPU/vector filtering, and one
      complete 54-segment loopback GSO injection.
- [ ] Calibration emits exactly one matching duration record with zero loss;
      the formal threshold is derived from that measured duration and the fresh
      formal run remains strictly over threshold.
- [ ] The formal run produces exactly one fixed softirq record and one
      correlated Error `HostIrqStorm` with `ControlledStop`, confidence 70,
      matching CPU/vector/cycle/timing fields, and one retained evidence item.
- [ ] Formal poll, statistics, correlator, and observer-health fields are exact
      and show one sample/overrun/emission with zero hard-IRQ activity,
      malformed/rejected/dropped evidence, or loss.
- [ ] Different hard-IRQ/softirq or CPU/vector identities do not merge into one
      incident, while same-identity evidence retains existing bounded merging.
- [ ] The qualification report is same-directory atomic and its validator is
      fail-closed for schema, integer ranges, filters, attach pair, calibration,
      GSO injection, counts, incident semantics, cycle identity, and health.
- [ ] Focused Rust/Python tests, formatting, Clippy, BPF syntax, capability
      validation, shell/Python syntax, `make ci`, and `git diff --check` pass.
- [ ] The dedicated privileged GitHub Actions job passes and its downloaded
      artifact independently passes the repository validator.
- [ ] Documentation removes controlled hosted loopback softirq injection from
      the open list while preserving hard-IRQ, production-NIC, production
      kernel, natural-load/root-cause, overhead/WCET, sustained-pressure, and
      long-duration limits.

## Non-Goals

- Do not claim hard-IRQ runtime qualification, NIC hardware IRQ identity,
  driver/NAPI behavior, external packet arrival, or Ethernet IRQ causality.
- Do not claim that the derived hosted threshold is a product threshold or that
  the loopback GSO burst represents production EtherCAT traffic.
- Do not change the fixed evidence size, evidence discriminants, attach bits,
  incident severity/action policy, MLG authority, or realtime control path.
- Do not qualify production kernels, PREEMPT_RT behavior, sustained interrupt
  pressure, overhead/WCET, CPU isolation, or long-duration HIL.
- Do not convert missing UDP GSO support, tracepoints, BPF permission, affinity,
  root/passwordless sudo, or exact counts into a skipped qualification.

## Technical Notes

- Linux `NET_RX` is softirq vector 3. `UDP_SEGMENT` is passed as an
  `IPPROTO_UDP` control message with a 1200-byte segment size; 54 segments keep
  the aggregate datagram below the IPv4 UDP payload limit.
- CPU selection samples `/proc/softirqs` over a bounded quiet window and chooses
  the allowed `u16` CPU with the smallest `NET_RX` delta. The BPF filter, not
  the `/proc` delta alone, owns evidence isolation.
- The calibration runtime and agent are discarded before the formal run so its
  counters, ringbuf records, incidents, and health transition start clean.
- The interrupt filter constants use `u16::MAX` for all CPUs and `u32::MAX` for
  all vectors. Existing deployments therefore retain the unfiltered behavior.
- Existing interrupt evidence records the task context in which the handler
  executes. For this bounded loopback burst the qualifier requires its own
  PID/TID, but documentation must not present that context as IRQ ownership or
  root-cause attribution.
