# Design: eBPF CPU frequency limit attribution

## Data flow

```text
power:cpu_frequency_limits
  -> typed {max_freq, cpu_id} read
  -> optional policy-CPU filter
  -> bounded per-policy episode state
  -> first max_freq < configured floor transition
  -> fixed 96-byte RuntimeEvidence
  -> Aya decoder
  -> IncidentCorrelator + transport-risk CycleContext
  -> HOST_CPU_THROTTLE
```

## Kernel contracts

- Add a 256-entry hash/LRU map keyed by policy CPU. Its value stores the last
  reported maximum frequency and whether the policy is currently below the
  configured floor.
- A missing state followed by a below-floor update starts and emits an
  episode. Further below-floor updates only update state and a suppression
  counter. An at/above-floor update clears the episode and increments a
  recovery counter; the next below-floor update may emit again.
- A map update failure increments `lost_events` and emits no evidence because
  the episode cannot be bounded reliably.
- `cpu_frequency_floor_khz`, `cpu_frequency_policy_cpu`, and an internal policy
  epoch are appended to the context ABI. `u32::MAX` means all policy CPUs;
  every other value is exact, including zero. An atomic runtime policy update
  increments the epoch so stale map entries cannot suppress the first event
  under a new floor or CPU filter.
- Statistics append observed updates, emitted episodes, recoveries, and
  suppressed updates. Existing field offsets remain unchanged.
- The tracepoint-provided policy CPU, rather than the CPU executing the hook,
  is written to the event. PID/TID remain zero.

## ABI and userspace

- The ring-buffer record remains 96 bytes and keeps `CpuThrottle = 6`.
- `observed_value` is policy `max_freq` in kHz; `threshold` is the configured
  floor in kHz; `count` is one for an emitted episode; `duration_ns` is zero
  because the tracepoint does not prove residency.
- Add one optional attach bit and `AttachPoint` entry. It is enabled by
  default but is not part of the default required mask.
- Runtime configuration and the update method validate a nonzero floor before
  writing the complete context value to the array map.

## Correlation and safety

- Classification requires `0 < observed_value < threshold` plus a matching
  cycle with deadline, WKC, or DC risk. An unrelated policy event or a healthy
  cycle is retained only as raw observation and is not labeled a root cause.
- `HostCpuThrottle` remains soft evidence with the existing error severity,
  degradation recommendation, and bounded incident aggregation.
- No observation directly changes MLG, PDO, controlword, or permit state.

## Compatibility and rollback

- Existing evidence layout and discriminants do not move.
- Context and statistics fields are append-only; C/Rust size assertions and
  tests are release gates.
- Missing tracepoint support degrades only this optional capability.
- Rollback removes the optional program, attach bit, map, appended fields, and
  docs without changing any external Protobuf or ProcBuf ABI.

## Qualification boundary

Local tests prove policy validation, fixed ABI decoding, episode semantics at
the source level, and incident correlation. CO-RE compilation proves the typed
record can be relocated against the build BTF. Real policy-limit injection,
shared-policy topology, verifier/load behavior, event loss, runtime overhead,
and long-duration stability remain target-environment qualification work.
