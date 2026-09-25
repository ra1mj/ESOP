# Design: eBPF observability degradation runtime qualification

## Qualification data flow

```text
healthy capability snapshot + epoch 1
  -> production page_fault_user attach only
  -> bounded anonymous first-write batches without polling
  -> production 4 MiB ringbuf fills
  -> bpf_ringbuf_output failure increments per-CPU lost_events
  -> BpfRuntime::poll observes monotonic loss delta
  -> AgentHealth records sticky epoch-local loss
  -> Degraded heartbeat + dedicated event-loss fault
  -> healthy snapshot reapplied, degradation remains sticky
  -> drop runtime -> restart(epoch 2) -> Restarting heartbeat
  -> reload production object/maps with epoch 2
  -> complete snapshot -> Healthy heartbeat
  -> one new page-fault record accepted with zero new loss
  -> detach -> atomic JSON report -> strict validator -> CI artifact
```

## Health-state contract

- Add `FAULT_AGENT_EVENT_LOSS = 0x4542_1004` beside the existing capability,
  load, and attach faults.
- `record_event_loss(0)` is a no-op. A positive count saturating-adds into the
  epoch-local counter and forces every non-Failed state to Degraded. It assigns
  the event-loss fault unless a stronger existing failure is already present.
- A healthy capability snapshot checks the current epoch-local loss count. If
  nonzero, state remains Degraded with the event-loss fault. Capability and
  hard-load failures still take precedence when their snapshot is incomplete.
- `restart(new_epoch)` clears heartbeat sequence, attach mask, loss count,
  incident count, fault, and last-event time; state becomes Restarting. The
  correlator already clears epoch-local incidents and adopts the new epoch.
- Restart does not itself claim kernel capability. Only applying the freshly
  loaded runtime's snapshot can transition Restarting to Healthy.

## Capability classification contract

- Reuse `CapabilitySnapshot::new`, `attach_ready`, and `hard_load_failure` as
  the single policy source.
- Unit tests construct snapshots for missing BTF, ringbuf, required attach,
  verifier, and permission. They prove state/fault classification, not host
  environment mutation.
- The runtime report records the actual hosted preflight and both loaded
  snapshots separately from the synthetic policy matrix.

## Saturation contract

- The example runs only on Linux x86_64 and requires effective root. It enables
  and requires exactly `ATTACH_PAGE_FAULT`, tracks its own TGID, sets threshold
  one/window one nanosecond, and uses unique nonzero boot/epoch identities.
- Allocate one fixed anonymous chunk (initially 64 MiB), write one byte per
  system page, unmap, and repeat. A fixed maximum total page count bounds time
  and work. Inspect statistics between batches without polling records.
- Stop after `lost_events > 0`. Require `page_faults > 0`, `emitted_events > 0`,
  `page_fault_threshold_events > 0`, and positive loss. Do not require an exact
  emitted count because incidental faults and ringbuf record alignment vary.
- Call `poll(..., 1)` once. Its `newly_reported_lost_events` must equal the
  saturated kernel loss clamped to `u32`, and the agent heartbeat must carry
  the same count and event-loss fault.

## Restart and recovery contract

- Reapply epoch-1 snapshot after loss and require no health recovery.
- Drop runtime 1 before calling `agent.restart(epoch_2)`. The first heartbeat
  must be Degraded because Restarting projects conservatively, with attach,
  loss, incident, and fault fields zero.
- Load runtime 2 from the same production object with epoch 2 and the same
  page-fault-only policy. Apply its complete snapshot and require Healthy.
- Trigger a bounded small anonymous first-write, poll until at least one record
  is seen, and require zero new loss, zero malformed/rejected evidence, and an
  epoch-2 Healthy heartbeat.

## Report and validator

- Use a flat closed JSON schema matching the existing qualification artifacts.
  Record architecture/page/ringbuf/chunk bounds, capability masks and policy
  matrix outcomes, both runtime snapshots, saturation counters, poll loss,
  all four health snapshots, recovery record counts, and cleanup flags.
- Delete stale final and temporary files before setup. Write the report only
  after both runtimes have been dropped, then rename a same-directory temp file
  atomically.
- The validator derives all expected masks, fault codes, state values, count
  relationships, epoch ordering, and cleanup invariants rather than trusting a
  boolean `qualified` field.

## Compatibility and rollback

- Production BPF maps and fixed ABIs remain unchanged. The only production
  semantic change is stronger `AgentHealth` epoch/loss behavior.
- The example is host-only and uses existing `libc`; no realtime crate gains a
  dependency.
- Rollback removes the qualification harness/CI/docs and reverts the health
  semantics/tests without touching evidence ABI or BPF attachment contracts.

## Qualification boundary

The artifact proves one hosted x86_64 production ring-buffer saturation and
runtime unload/reload path. It does not prove target-kernel behavior, missing
BTF/permission environments, exact capacity, production traffic pressure,
resource overhead, WCET, or long-duration stability.
