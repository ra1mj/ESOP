# eBPF page-fault runtime qualification

## Goal

Qualify the existing FR-048/EBPF-005 tracked-process page-fault count window
on a privileged hosted x86_64 Linux kernel. The evidence must prove that the
production CO-RE object can load and require `exceptions:page_fault_user`,
observe an exact bounded set of controlled anonymous-page first writes from a
prepared child process, emit the first threshold crossing, correlate it with a
transport-risk cycle, and project the resulting `HOST_PAGE_FAULT` incident
into observer health.

## Background

The repository already filters `page_fault_user` by tracked TGID, aggregates
counts in a fixed 256-entry LRU map keyed by CPU/TGID, emits only the first
threshold crossing in a bounded window, preserves the triggering PID/TID and
architecture error code, decodes the fixed 96-byte record, and correlates it
with cycle risk as Warning `HOST_PAGE_FAULT` / `DegradeHostObservation` with
confidence 65. Source tests and CO-RE compilation do not establish real
verifier/load behavior, tracepoint availability, isolated fault injection,
ringbuf delivery, exact kernel statistics, or end-to-end health projection.

Linux defines `page_fault_user` from the generic exception trace event with
address, instruction pointer, and architecture error code. The existing ABI
deliberately exports only the saturated error code; it does not expose the
address, classify major versus minor faults, or measure handler duration.

## Requirements

- Add a Linux-only `page_fault_qualification` example that starts a separate
  child mode before BPF loading, pins it to one representable allowed CPU,
  prepares an untouched anonymous mapping, warms the exact control/touch code
  path, then blocks with all setup complete.
- Load the production BPF object only after the child reports readiness, with
  enabled and required attach masks both exactly `ATTACH_PAGE_FAULT`, tracked
  PID equal to the child TGID, threshold 16, and a one-second count window.
  Unrelated programs must remain disabled.
- Publish the same deadline-risk `CycleContext` to the runtime and
  `RuntimeAgent` before releasing the child. The agent must first report a
  Healthy heartbeat and become Degraded with fault `0x45422001` after the
  incident is correlated.
- Release the child through an already-warmed pipe operation. The child must
  first-write exactly 16 distinct system pages while pinned to the target CPU,
  report its `getrusage(RUSAGE_SELF).ru_minflt` delta, and keep all post-load
  communication in pre-faulted memory. Require the child minor-fault delta to
  equal 16 and the child to exit successfully.
- Require baseline statistics to be zero for emitted/lost events, page faults,
  and page-fault threshold events before release.
- Require exactly one `KernelMemory/PageFault/Warning` record with child
  PID/TID, target CPU, zero IRQ/ifindex, the injected cycle identity,
  `observed_value == threshold == count == 16`, positive bounded window
  duration, x86 page-fault detail 6 (user + write + not-present), and nonzero
  evidence ID/timestamp.
- Require exactly one Warning `HostPageFault` incident with
  `DegradeHostObservation`, confidence 65, matching child/CPU/cycle/count and
  timestamp fields, one retained evidence item, and no dropped incident.
- Require exact final kernel statistics: `page_faults=16`,
  `page_fault_threshold_events=1`, `emitted_events=1`, and zero loss. Polling
  must report one record, one incident, and zero malformed, rejected, or newly
  reported lost records.
- Remove stale final/temp reports at start and atomically publish
  `build/ebpf_page_fault_qualification.json` only after every assertion passes.
  Add a strict allowlist validator and regression tests for schema and semantic
  failures.
- Add Make targets and a dedicated privileged GitHub Actions job. Build BPF and
  Rust without elevation, elevate only the final fixture, validate as the
  invoking user, and upload the validated report.
- Update README, software PRD, runtime observability design, capability
  manifest, and backend quality guidance to claim only the controlled hosted
  anonymous-page first-write chain demonstrated by the report.

## Acceptance Criteria

- [x] A privileged hosted x86_64 Linux run verifies production CO-RE load and
      required `exceptions:page_fault_user` attachment with no unrelated hook.
- [x] A prepared child remains fault-quiet after BPF load until release, then
      records exactly 16 minor faults from 16 distinct anonymous-page writes on
      one pinned CPU.
- [x] Baseline counters are zero; the formal run produces exactly one fixed
      page-fault record and exact final page-fault/emission/loss statistics.
- [x] The record preserves child PID/TID, target CPU, injected cycle identity,
      threshold/count 16, positive in-window duration, and x86 error detail 6.
- [x] Correlation produces exactly one Warning `HostPageFault` with
      `DegradeHostObservation`, confidence 65, and a Healthy-to-Degraded
      heartbeat transition with fault `0x45422001`.
- [x] The report is same-directory atomic and its validator is fail-closed for
      schema, integer ranges, attach mask, child/CPU identity, fault counts,
      window timing, x86 detail, incident semantics, cycle identity, loss, and
      observer health.
- [x] Focused Rust/Python tests, formatting, Clippy, BPF syntax, capability
      validation, shell/Python syntax, `make ci`, and `git diff --check` pass.
- [x] The dedicated privileged GitHub Actions job passes and its downloaded
      artifact independently passes the repository validator.
- [x] Documentation removes controlled hosted anonymous-page first-write
      injection from the open list while preserving major/minor attribution,
      fault address, handler duration, natural workload/root-cause, production
      threshold/kernel, overhead/WCET, and long-duration limits.

## Non-Goals

- Do not classify major versus minor faults from the exception tracepoint or
  claim that the BPF record itself proves the child `ru_minflt` classification.
- Do not expose fault address/IP, measure page-fault handler duration, or change
  the fixed event/context/map ABI, evidence discriminants, attach bits,
  incident severity/action policy, MLG authority, or realtime control path.
- Do not claim memory-pressure behavior, swap/storage latency, NUMA, COW,
  allocator behavior, production thresholds, production kernels, overhead,
  WCET, sustained pressure, or long-duration HIL.
- Do not convert missing tracepoints, BPF permission, affinity, root/passwordless
  sudo, x86_64 architecture, exact fault counts, or exact detail bits into a
  skipped qualification.

## Technical Notes

- The child is spawned and fully prepared before the runtime is loaded. The
  parent knows its PID in advance and can set `tracked_pid` in the initial
  context, avoiding policy updates after attach.
- The child uses a raw fixed-buffer control protocol. It performs a dry-run on
  a separate mapping before readiness so executable pages, stack, pipe buffers,
  affinity code, and `getrusage` structures are resident before tracking.
- Anonymous pages are mapped without `MAP_POPULATE`; one volatile write per
  system page produces the intended non-present write fault. The page size and
  target CPU are reported and validated.
- On x86, a user-mode write to a non-present page has error-code bits
  `X86_PF_USER | X86_PF_WRITE = 6`. This is a hosted x86_64 qualification
  contract, not a portable architecture-neutral interpretation.
- The BPF `duration_ns` field is elapsed count-window time from the first to the
  threshold-triggering fault, not page-fault handler duration.
