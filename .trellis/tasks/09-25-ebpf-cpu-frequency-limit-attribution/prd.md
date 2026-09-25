# eBPF CPU frequency limit attribution

## Goal

Complete the bounded Linux kernel-to-incident path for CPU frequency policy
limits so ESOP can identify a policy cap below the product-qualified frequency
floor when it overlaps a degraded EtherCAT cycle.

## Background

FR-048 names CPU throttling as a required Linux observation. The shared event
and incident ABIs already reserve `CpuThrottle` and `HostCpuThrottle`, but the
BPF bundle has no producer for that evidence. Linux `power:cpu_frequency_limits`
reports the policy `min_freq`, `max_freq`, and `cpu_id` whenever cpufreq policy
limits change. This is a policy-limit signal, not proof of the instantaneous
clock, thermal cause, power-budget cause, or time spent throttled.

## Requirements

- Observe the optional `power:cpu_frequency_limits` tracepoint through its
  typed context and read `max_freq` plus the policy `cpu_id`.
- Expose a nonzero product-qualified frequency floor in kHz and an optional
  policy-CPU filter through `RuntimeConfig`, the C/Rust kernel context ABI, and
  an atomic runtime update method. Use an explicit all-CPU sentinel so CPU 0
  remains selectable.
- Treat only a positive policy `max_freq` below the configured floor as
  `CpuThrottle` evidence. Ordinary frequency transitions and policy limits at
  or above the floor must not produce incidents.
- Keep one fixed-capacity state entry per observed policy CPU. Emit one fixed
  96-byte record on the first below-floor observation, suppress further
  below-floor limit changes in the same episode, and rearm only after a policy
  update at or above the floor.
- Store the affected policy CPU in the evidence `cpu` field, the reported
  policy maximum in `observed_value`, and the configured floor in `threshold`.
  Do not attribute a task PID/TID to this system policy event.
- Extend per-CPU statistics with observed policy updates, emitted below-floor
  episodes, recovery/rearm transitions, and suppressed below-floor updates.
  Rust aggregation must remain saturating.
- Classify `CpuThrottle` as `HOST_CPU_THROTTLE` only when the maximum is below
  the floor and the evidence overlaps an existing deadline/WKC/DC-risk cycle.
  Healthy-cycle or malformed threshold evidence must not be called a root
  cause.
- Keep the attach point optional in the default required mask so kernels or
  platforms without cpufreq tracing report reduced capability without
  breaking the scheduler/process baseline.
- Preserve the existing evidence discriminants and fixed 96-byte ring-buffer
  ABI. Keep eBPF observation outside the realtime control path and unable to
  write MLG, PDO, controlword, or motion-permit state.
- Update README, software PRD, runtime observability design, capability
  manifest, and backend quality guidance without claiming thermal-cause,
  instantaneous-frequency, residency, or target-kernel qualification.

## Acceptance Criteria

- [ ] The BPF bundle contains a bounded policy-CPU state map and a typed
      `power:cpu_frequency_limits` program.
- [ ] The first below-floor policy update emits one record, repeated
      below-floor changes are suppressed, and an at/above-floor update rearms
      the next episode.
- [ ] The fixed event decodes the affected policy CPU, maximum frequency, and
      configured floor without changing its 96-byte size.
- [ ] Runtime configuration rejects a zero floor and invalid CPU-filter
      combinations without partially mutating the kernel context.
- [ ] Unit tests prove `HOST_CPU_THROTTLE` requires both a below-floor maximum
      and a correlated transport-risk cycle.
- [ ] C and Rust agree on expanded context/stat map sizes, attach masks remain
      explicit, and statistics aggregation tests cover saturation.
- [ ] `make bpf-syntax` and `make ci` pass locally.
- [ ] GitHub Actions builds the real CO-RE BPF object and passes the Rust gate.

## Non-Goals

- Do not interpret ordinary `power:cpu_frequency` transitions as throttling.
- Do not infer thermal, powercap, firmware, governor, cgroup, or hardware
  causes from a limit update that does not contain those facts.
- Do not claim the policy maximum is the instantaneous CPU frequency or prove
  how long the limit remained active.
- Do not add task migration tracking, cpuset/cgroup attribution, thermal-zone
  probes, frequency residency histograms, or sysfs polling in this increment.
- Do not claim EBPF target-kernel verifier, limit injection, event-pressure,
  overhead, or long-duration qualification from source and unit tests.

## Notes

- `cpu_frequency_limits` identifies a cpufreq policy by its representative
  `cpu_id`; shared policies may cover more than one logical CPU. The evidence
  therefore states the kernel policy identity, not complete affected-cpu
  membership.
