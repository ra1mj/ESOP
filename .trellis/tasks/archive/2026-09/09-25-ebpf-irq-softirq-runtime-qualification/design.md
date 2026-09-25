# Design: eBPF softirq runtime qualification

## Qualification data flow

```text
allowed affinity + /proc/softirqs quiet sample
  -> select and pin one target CPU
  -> production BPF object attaches only softirq entry + exit
  -> kernel context filters exact target CPU + NET_RX vector 3
  -> calibration UDP GSO burst -> measured duration -> unload
  -> derive threshold = max(1, calibration / 8)
  -> fresh runtime + fresh RuntimeAgent + shared risk cycle
  -> formal UDP GSO burst -> one fixed ringbuf record
  -> Aya decoder -> keyed HostIrqStorm correlator -> degraded heartbeat
  -> atomic JSON report -> strict validator -> CI artifact
```

## Interrupt filter contract

- Rename `KernelContext.reserved16` to `interrupt_filter_cpu` and
  `reserved32` to `interrupt_filter_vector`; mirror the names in the BPF C
  context. Layout and size remain unchanged.
- Add `RuntimeConfig::{interrupt_filter_cpu, interrupt_filter_vector}` with
  defaults `u16::MAX` and `u32::MAX`. These sentinels mean no filter.
- Hard-IRQ and softirq entry programs read the context before map insertion.
  A nonmatching CPU or vector returns without state or statistics. Exit probes
  naturally ignore filtered entries because no start timestamp exists.
- `update_interrupt_filter` writes both values atomically through the existing
  whole-context array update. Cycle updates preserve them.

## Incident identity contract

- `HOST_IRQ_STORM` merge identity includes evidence kind, CPU, and vector in
  addition to boot/epoch/window. This prevents hard IRQ vector 3, softirq
  vector 3, and the same vector on different CPUs from becoming one incident.
- The first retained evidence item is the authoritative interrupt class for
  an existing incident. The change does not affect other incident codes.

## Injection protocol

- Read the real process affinity and keep only CPU IDs representable by `u16`.
  Read `NET_RX` per-CPU counts from `/proc/softirqs`, wait a short bounded quiet
  interval, then choose the allowed CPU with the smallest delta.
- Pin the fixture thread to the chosen CPU and verify `sched_getcpu`.
- Create connected sender/receiver IPv4 UDP sockets on loopback. Increase the
  receive buffer and set bounded receive timeout.
- Build one 64,800-byte payload and issue `sendmsg` with `UDP_SEGMENT=1200`,
  yielding 54 receive datagrams. Require the full aggregate send length and
  drain exactly 54 datagrams of 1200 bytes.
- Record target-CPU `NET_RX` counters around injection as supporting evidence,
  but require BPF CPU/vector fields and exact filtered BPF statistics for the
  qualification claim.

## Calibration and formal runtime

- A helper runs one isolated runtime phase. Both phases load the production
  object with only the softirq pair enabled/required and the exact CPU/vector
  filter.
- Calibration uses threshold 1 ns and a risk cycle only to recover the measured
  target evidence through the existing correlator. It requires one record,
  one incident, one sample, one overrun, one emission, and zero loss.
- The formal phase uses `max(1, calibration_duration / 8)`, a fresh runtime,
  fresh sockets, and a fresh agent. It publishes the formal cycle to both
  runtime and agent before injection and captures Healthy state first.
- A duration at or below the derived threshold, extra filtered samples, extra
  records, missing datagrams, or any loss fails the run. There is no retry that
  can silently weaken the threshold or count contract.

## Evidence, incident, and health contracts

- Evidence is Error `KernelIrq/SoftirqCpuTime`, with the exact fixture PID/TID,
  zero ifindex/detail, exact target CPU, `irq=3`, count one, and identical positive
  `observed_value`/`duration_ns` strictly above the derived threshold.
- Correlation produces Error `HostIrqStorm`, action `ControlledStop`, confidence
  70, and one retained evidence item. Capability projection first makes the
  formal agent Healthy; the incident changes it to Degraded with fault
  `0x45422001`.
- The fixture PID/TID identifies the task context that returned through the
  inline softirq path. It is supporting identity only and is not presented as
  ownership of the softirq or as causal task attribution.
- Formal statistics are exact because only one CPU/vector pair is admitted:
  one softirq sample/overrun/emission, zero hard-IRQ samples/overruns, and zero
  loss. Calibration statistics are recorded separately and obey the same exact
  contract.

## Report and validation

- Delete stale final/temp reports before setup. Write the full report to a
  same-directory temporary path and rename only after every runtime assertion.
- The closed schema records CPU selection, quiet-window counters, GSO/socket
  parameters, calibration/final thresholds and durations, attach/filter
  values, baseline/final statistics, poll fields, full formal incident/evidence,
  and initial/final observation states.
- The validator rejects booleans as integers, unknown/missing fields, invalid
  CPU/vector/filter values, partial attach pairs, invalid calibration formula,
  incomplete GSO send/receive, nonexact counters, duration inconsistencies,
  wrong classification/cycle fields, loss, and health mismatches.

## Compatibility and safety

- The context ABI reuses existing reserved slots, so C/Rust size and downstream
  map layout remain stable. All-filter sentinels preserve current behavior.
- The qualification is a Linux-only example and reuses the runtime crate's
  existing `libc` development dependency. No production crate gains a socket
  or thread dependency.
- BPF/Rust compilation and report validation remain unprivileged. Only the
  prebuilt fixture is elevated for BPF loading; socket and affinity operations
  themselves do not require privilege.

## Operational limits and rollback

- Hosted CI proves one controlled loopback `NET_RX` softirq duration chain on
  its kernel. It does not qualify a hard IRQ, a NIC/driver/NAPI path, product
  interrupt budgets, production kernels, sustained pressure, overhead, or WCET.
- Rollback removes the example, runner, validator/tests, Make/CI targets, docs,
  filter API, and keyed merge rule. The BPF context size remains unchanged in
  either direction.
