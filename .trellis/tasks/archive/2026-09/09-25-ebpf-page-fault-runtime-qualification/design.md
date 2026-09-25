# Design: eBPF page-fault runtime qualification

## Qualification data flow

```text
allowed affinity -> choose one u16 CPU
  -> exec child, pin CPU, warm control/touch path, map 16 untouched pages
  -> child reports ready and blocks in pre-faulted control buffer
  -> production BPF object attaches only page_fault_user for child TGID
  -> publish shared deadline-risk cycle and Healthy heartbeat
  -> release child -> 16 first writes -> one threshold crossing
  -> child reports ru_minflt delta and blocks
  -> ringbuf decode -> HostPageFault correlation -> degraded heartbeat
  -> detach BPF -> release child cleanup and exit
  -> atomic JSON report -> strict validator -> CI artifact
```

## Child isolation protocol

- The parent selects one CPU from its real allowed affinity whose ID fits the
  evidence `u16` field and passes it to an exec child mode.
- The child pins itself to that CPU, verifies `sched_getcpu`, allocates two
  anonymous mappings, and touches a warm-up mapping through the same helper
  used by the formal injection. The formal 16-page mapping remains untouched.
- The child pre-faults fixed command/result buffers and calls the raw read,
  write, affinity, `getrusage`, and unmap paths before readiness. It then sends
  a fixed readiness record and blocks on a one-byte release command.
- After release, the child samples `ru_minflt`, performs one volatile byte write
  per formal page, samples `ru_minflt` again, writes a fixed binary result, and
  exits. No allocator, formatting, thread creation, or buffered I/O is used in
  the tracked phase.
- Parent-side deadlines cover readiness, result delivery, process exit, and BPF
  polling. RAII kills and reaps an incomplete child on any error.

## Runtime and correlation contract

- The runtime uses only `ATTACH_PAGE_FAULT` in enabled and required masks,
  `tracked_pid=child_pid`, threshold 16, and a one-second window. Existing
  production BPF, decoder, statistics, and correlator code remain unchanged.
- A deadline-miss cycle with stable boot/epoch/cycle/transition IDs is written
  to both the kernel context and the agent immediately before release.
- The fixture checks a zero baseline after load. The tracked child must not
  produce any page fault while blocked; any setup leak fails before injection.
- The BPF key is `(target_cpu, child_tgid)`. CPU pinning prevents the 16 faults
  from splitting across per-CPU windows.
- The sixteenth fault emits one record. Its `duration_ns` is positive and less
  than the configured one-second window; it is not interpreted as handler
  duration.
- The correlator requires the count threshold and cycle risk, producing Warning
  `HostPageFault`, action `DegradeHostObservation`, confidence 65. The runtime
  health projection moves from Healthy to Degraded with fault `0x45422001`.

## Evidence and statistics contract

- Evidence identity is the child leader PID/TID and selected CPU. IRQ and
  ifindex are zero. Domain/kind/severity are KernelMemory/PageFault/Warning.
- Count, observed value, and threshold are all 16. The x86 detail byte is 6,
  proving the triggering exception was user-mode, write, and not-present at the
  architecture error-code level.
- `ru_minflt_delta=16` is independent supporting process accounting. It is not
  used to reinterpret the BPF event as a portable major/minor classifier.
- Exact final statistics are 16 page faults, one threshold event, one emitted
  event, and zero lost events. One poll record and one retained incident are
  required, with no malformed/rejected evidence or dropped incident.

## Report and validation

- Delete stale final/temp reports before child setup. Write the complete report
  to a same-directory temporary path and rename only after all assertions pass.
- The closed schema records architecture, page size/count, target CPU, child
  readiness/result values, attach/window configuration, baseline/final stats,
  poll fields, complete incident/evidence identity, cycle fields, and initial/
  final observer health.
- The Python validator rejects booleans as integers, unknown/missing keys,
  non-x86_64 reports, bad page geometry, partial/wrong attach masks, mismatched
  child/CPU/cycle fields, nonexact counts, bad window timing or detail bits,
  wrong incident policy, loss, and health mismatches.

## Compatibility and safety

- This task adds a qualification harness only. It does not alter BPF maps,
  program logic, event/context sizes, runtime APIs, or incident policy.
- The example is Linux-only and reuses the runtime crate's existing `libc`
  development dependency. Production crates gain no new dependency.
- BPF/Rust compilation and report validation remain unprivileged. Only the
  prebuilt fixture is elevated for BPF loading and tracepoint attachment.

## Operational limits and rollback

- Hosted CI proves one controlled anonymous-page first-write chain on its
  x86_64 kernel. It does not qualify major faults, storage/swap, production
  workload causality, handler duration, product thresholds, production kernels,
  overhead, or WCET.
- Rollback removes the example, runner, validator/tests, Make/CI targets, docs,
  and capability evidence entry. Production runtime behavior is unaffected.
