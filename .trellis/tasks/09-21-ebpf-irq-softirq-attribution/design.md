# Design

## Data Flow

```text
irq/softirq entry tracepoint
  -> bounded {cpu, vector} -> start_ns map
irq/softirq exit tracepoint
  -> duration calculation + per-CPU counters
  -> fixed 96-byte ringbuf evidence when duration > threshold
  -> Rust decoder
  -> RuntimeAgent correlator + current CycleContext risk window
  -> HOST_IRQ_STORM incident
```

## BPF Contracts

- `ESOP_IRQ_STARTS`: hash map, maximum 1024 entries, key `{cpu, vector}`.
- `ESOP_SOFTIRQ_STARTS`: hash map, maximum 256 entries, key `{cpu, vector}`.
- Entry overwrites a stale value for the same key. A map update failure is
  counted as a lost event because duration evidence can no longer be formed.
- Exit deletes the start entry before optional ringbuf emission.
- Missing starts are ignored; they can occur when attaching after an entry or
  when an earlier bounded-map update failed.
- Each sample increments its class sample counter. An emitted sample also
  increments its class overrun counter.
- The existing event layout remains 96 bytes. `irq` carries the vector as a
  saturating `u16`; hard IRQ uses `IrqCpuTime = 1`, softirq uses the new
  `SoftirqCpuTime = 9` discriminant.

## Rust Loader Contracts

- Attach bits 6-9 represent hard IRQ entry/exit and softirq entry/exit.
- `ATTACH_ALL` covers all ten declared points.
- `RuntimeConfig::valid` owns attach-mask, threshold, and pair validation.
- Default enabled points include the new probes, while the required mask stays
  scheduler wakeup/switch plus process exit.
- `KernelContext` grows from 56 to 72 bytes with two `u64` thresholds.
- `KernelStats` grows from 56 to 88 bytes with four `u64` counters.
- `update_irq_thresholds` changes only interrupt thresholds and writes the
  whole map value atomically through Aya's array-map update.

## Correlation Contract

- Hard IRQ and softirq duration are soft evidence, not standalone root cause
  proof.
- `classify` maps either evidence kind to `HostIrqStorm` only when duration is
  over threshold and `IncidentCorrelator` has a matching transport-risk cycle
  in the configured time window.
- Incidents retain the source evidence kind and vector, so consumers can tell
  hard IRQ from softirq without changing the incident code.

## Failure Behavior

- Invalid pair configuration fails closed before object load.
- Missing optional tracepoints remain absent from `attach_mask`; the capability
  snapshot exposes the reduction.
- IRQ and softirq tracepoints attach as pairs. The loader attaches exit before
  entry and rolls the first link back if the second member fails, so an
  optional half-pair cannot keep filling a start-time map.
- Required attach failures still abort loading.
- Ringbuf/map pressure increments existing loss telemetry and never blocks the
  realtime path.
- No observation result grants motion or clears a lifecycle fault.

## Compatibility

- Existing evidence offsets and total size do not change.
- Existing evidence discriminants 0-8 remain stable; 9 is append-only.
- Existing `update_tracking` callers remain source-compatible.
- The fallback `bpf/vmlinux.h` gains only the tracepoint structures required by
  local syntax checks; production builds still generate `vmlinux.h` from BTF.
