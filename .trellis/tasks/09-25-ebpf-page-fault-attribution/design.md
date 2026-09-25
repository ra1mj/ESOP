# Design: eBPF page-fault window attribution

## Data flow

```text
exceptions:page_fault_user
  -> typed address/ip/error-code read
  -> tracked process filter
  -> bounded {cpu, tgid} window state
  -> first threshold-crossing 96-byte RuntimeEvidence
  -> Aya decoder
  -> IncidentCorrelator + cycle-risk CycleContext
  -> HOST_PAGE_FAULT
```

## Kernel contracts

- `esop_context` adds `page_fault_threshold` and
  `page_fault_window_ns`. C and Rust definitions remain fixed-size with
  explicit size assertions.
- `ESOP_PAGE_FAULTS` is a 256-entry LRU hash. Its key is CPU plus process ID;
  its value is `window_start_ns` plus a saturating 32-bit count. Per-CPU keys
  avoid concurrent mutation of one value while retaining process attribution.
- A missing or expired entry begins at count one. The event that makes
  `count == threshold` emits once; later faults in the same window only update
  counters. Window expiry rearms the threshold.
- Map insertion failure increments `lost_events`. Every tracked fault
  increments `page_faults`; successful threshold evidence increments
  `page_fault_threshold_events`.
- The tracepoint's architecture-specific error code is saturated to the final
  one-byte `detail` field. Address and IP remain available in the typed context
  contract but are not forced into unrelated fixed event fields.

## ABI and userspace

- The ring-buffer event remains exactly 96 bytes. For `PageFault`,
  `observed_value` and `count` carry the window count, `threshold` carries the
  configured count, `duration_ns` carries elapsed window time, and `detail`
  carries the low/saturated error code.
- `RuntimeConfig` defaults to threshold one and a 1 ms window, preserving the
  existing "any fault in the tracked RT process is suspicious" policy while
  bounding emission to one record per CPU/process/window.
- A dedicated `update_page_fault_tracking` method validates both fields and
  commits them through one context-map write.
- `KernelStats` adds `page_fault_threshold_events`; userspace aggregation
  remains saturating.

## Correlation

- `PageFault` remains soft evidence. It becomes `HOST_PAGE_FAULT` only when
  the evidence count reaches the configured threshold and the active
  `CycleContext` reports deadline, WKC, or DC risk inside the correlation
  window.
- Process exit and OOM remain hard facts and are unchanged.
- Incident merging keeps the existing process identity and bounded raw
  evidence list; the detail byte remains available for diagnostics.

## Compatibility and rollback

- `page_fault_user` remains optional in the default required mask. Missing
  tracepoint support or optional-program load failure produces a reduced
  attach mask rather than blocking the required scheduler/process path.
- No ProcBuf, MLG, EtherCAT core, gateway, or motion-control ABI changes are
  required.
- Rollback removes the new map/context/stat fields while preserving the
  existing evidence discriminant and attach bit.

## Qualification boundary

Local tests prove policy validation, fixed ABI decoding, and correlation.
GitHub CI proves the checked source compiles as a CO-RE object against its
runner kernel BTF. Real target-kernel loading, verifier behavior, page-fault
injection, ring-buffer pressure, and runtime overhead remain EBPF-005/EBPF-010
environment-level evidence.
