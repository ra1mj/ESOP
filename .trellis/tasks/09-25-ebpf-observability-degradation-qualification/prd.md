# eBPF observability degradation runtime qualification

## Goal

Qualify the FR-047/FR-050 and EBPF-008 observability-degradation path on a
privileged hosted Linux kernel. The evidence must prove that the production
CO-RE object can load and require `exceptions:page_fault_user`, saturate its
non-blocking ring buffer without polling, expose kernel-side loss through
statistics, project the loss into a fail-closed `HostObservation`, and recover
only through a new agent epoch and a newly loaded runtime.

## Background

The BPF bundle already increments `lost_events` when ring-buffer output or a
bounded map update fails. `BpfRuntime::poll` converts the monotonic kernel loss
delta into `AgentHealth::record_event_loss`, and the lifecycle guard treats a
Degraded host observation as a failed motion gate. Source tests do not prove
that the production ring buffer can be saturated, that the loss delta crosses
the BPF/Rust boundary, or that unload/reload resets the epoch-local counters.

Two health semantics also need to be explicit. Event loss currently changes
the state to Degraded without a dedicated fault code, and applying a healthy
capability snapshot can return the same epoch to Healthy even though its loss
counter remains nonzero. A new epoch should begin in Restarting with no stale
attach mask, counters, or fault, and should become Healthy only after the new
runtime reports a complete capability snapshot.

## Requirements

- Add a dedicated event-loss fault code and make any nonzero event loss sticky
  for the current agent epoch. Reapplying a healthy capability snapshot in the
  same epoch must not clear Degraded state while the loss count is nonzero.
- `RuntimeAgent::restart` must reset epoch-local attachment, event-loss,
  incident, and fault state while preserving the boot identity. Its first
  heartbeat must be Degraded/Restarting with no stale runtime capability.
- A complete capability snapshot after restart may restore Healthy state. A
  missing BTF/ringbuf/required attach point remains Degraded, while missing
  verifier or BPF permission remains Failed. These classifications must have
  deterministic unit coverage without claiming the hosted runner lacks them.
- Add a Linux/x86_64-only `observability_degradation_qualification` example
  that requires root and BPF privileges, loads only
  `exceptions:page_fault_user`, and configures the current TGID with threshold
  one and a minimal nonzero window so every bounded anonymous first-write can
  attempt one fixed event.
- Saturate the production 4 MiB ring buffer without polling by repeatedly
  mapping, first-writing, and unmapping a bounded memory chunk. Stop at the
  first observed kernel loss, and fail if a fixed maximum page count is
  reached without loss.
- Require the saturation phase to increase page-fault and emitted-event
  counters, produce a positive kernel `lost_events` count, and make one bounded
  poll report the same newly observed loss into `AgentHealth`.
- Require the post-loss heartbeat to be Degraded with the dedicated event-loss
  fault and the exact saturated loss count. Reapplying the healthy snapshot in
  the same epoch must remain Degraded with the same loss evidence.
- Drop the old runtime, restart the agent with a strictly larger epoch, verify
  the Restarting heartbeat has no stale attach mask/counters/fault, reload the
  production object with the new epoch, and require a Healthy heartbeat after
  applying the new complete snapshot.
- After reload, trigger and consume at least one fixed page-fault record and
  require zero new loss and zero evidence rejection, proving the new epoch's
  kernel/user-space evidence identity is accepted.
- Remove stale final/temp reports at start and atomically publish
  `build/ebpf_observability_degradation_qualification.json` only after both
  runtimes are detached and all assertions pass. Add a closed-schema validator
  and mutation-based regression tests.
- Add Make targets and a dedicated privileged GitHub Actions job. Build BPF
  and Rust without elevation, elevate only the fixture, validate as the
  invoking user, and upload the report artifact.
- Update README, software PRD, runtime-observability design, capability
  manifest, and backend quality guidance with only the hosted ring-buffer-loss
  and restart-recovery claim proven by the report.

## Acceptance Criteria

- [ ] Unit tests prove event loss has a dedicated fault, is sticky within one
      epoch, and is cleared only by restart into a new epoch.
- [ ] Capability tests distinguish missing BTF/ringbuf/required attach points
      from missing verifier/permission without depending on host mutation.
- [ ] A privileged hosted Linux run loads only `page_fault_user`, fills the
      production 4 MiB ring buffer, and records positive page-fault, emitted,
      and lost-event counters without blocking the producer.
- [ ] One poll reports the exact new kernel loss to the agent, and both the
      immediate and same-epoch re-applied heartbeat remain Degraded with the
      event-loss fault and matching count.
- [ ] Unload/restart/reload advances the agent epoch, clears old attachment and
      counters, restores Healthy only after a complete new snapshot, and
      accepts a new-epoch page-fault record with zero new loss.
- [ ] The report is same-directory atomic and its validator fails closed for
      schema, capability truth table, attach masks, saturation bounds,
      statistics, loss projection, restart reset, recovery, and cleanup.
- [ ] Focused Rust/Python tests, formatting, Clippy, BPF syntax, capability
      validation, shell/Python syntax, `make ci`, and `git diff --check` pass.
- [ ] The dedicated GitHub Actions job passes and its downloaded artifact
      independently passes the repository validator.
- [ ] Documentation removes only hosted ring-buffer-loss/restart qualification
      from the open list while preserving production pressure, permissions,
      missing-BTF environments, overhead/WCET, and long-duration limits.

## Non-Goals

- Do not reduce the production ring-buffer size or add a test-only BPF map.
- Do not use a synthetic lost-event setter as proof of kernel ring-buffer
  saturation; the hosted report must observe production BPF statistics.
- Do not claim the hosted runner actually exercised missing BTF, missing
  permission, verifier rejection, or required-attach absence. Those states are
  deterministic policy/model tests only.
- Do not claim a precise ring-buffer capacity in records; BPF ring-buffer
  metadata and alignment are implementation details. Require bounded positive
  emitted and lost counts instead.
- Do not change fixed evidence, context, statistics, Protobuf, or ProcBuf ABIs,
  evidence discriminants, attachment bits, MLG authority, or realtime paths.
- Do not claim production event-pressure, CPU/memory overhead, WCET, target
  kernel, sustained load, or 30-minute qualification.

## Technical Notes

- The checked-in ring buffer uses `max_entries = 1 << 22`. Keep this production
  size and generate enough fixed 96-byte page-fault records to exceed it.
- Use repeated bounded anonymous mappings rather than one unbounded allocation;
  first-write one byte per page, then unmap before the next batch.
- Configure `page_fault_threshold=1` and `page_fault_window_ns=1`. The tracked
  TGID and per-CPU/TGID state retain production filtering and map behavior.
- Poll only after `KernelStats.lost_events > 0`; reading per-CPU statistics does
  not consume the ring buffer.
- A runtime reload owns a fresh map set, so kernel statistics and event queue
  are epoch-local without adding reset APIs to production maps.
