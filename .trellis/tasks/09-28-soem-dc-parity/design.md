# Design: SOEM parity with DC preservation

## Capability model

Each parity item is an optional controller attached to
`ScheduledProductionServices`. It owns a fixed request table and publishes an
immutable status/result/fault projection. The scheduler remains the only
owner that turns a prepared action into a control or mailbox datagram.

```text
product intent -> generated fixed plan -> typed optional controller
  -> ScheduledProductionServices after cyclic Domain/DC work
  -> existing control/mailbox pool -> exact completion checks
  -> immutable status + diagnostic event -> lifecycle policy if configured
```

## Default priority

P0 Domain and cyclic DC always run first. Startup/recovery configuration keeps
its existing order. Runtime mailbox protocols share a bounded P2 budget;
register diagnostics remain lowest priority. FoE and EoE are maintenance or
explicitly budgeted traffic and cannot consume an unbounded number of frames
per cycle.

## DC coexistence

Every controller validates its datagram indices and image ranges against the
frozen Domain/DC plan at activation. Redundancy owns duplicate/reorder
suppression below the core RX admission boundary so the core sees at most one
authoritative response per expected generation. DC samples from the backup
path cannot be combined with a primary-path sample unless the redundancy
controller proves they are the same transmitted generation.

## Capability slices

- Cyclic mailbox: fixed per-slave queues, one in-flight owner, fair bounded
  polling, and protocol demultiplexing compatible with current CoE.
- FoE: bounded block buffers, maintenance-only transfer state, progress/result
  snapshots, and no firmware-success claim before product verification.
- SoE: typed IDN requests and optional mapping discovery through the shared
  mailbox layer.
- EoE: host-only fragmentation/reassembly and bounded queues; no virtual NIC in
  the real-time core.
- Redundancy: platform dual-port abstraction, generation-aware duplicate and
  reorder handling, link diagnostics, and isolated failover policy.
- Recovery/HIL: observation-driven policy above existing explicit rescan,
  reconfigure and state requests; lifecycle requalification remains mandatory.

## Rollout

Create a child task per slice, starting with cyclic mailbox handling after the
diagnostic CLI child is complete. Keep absent features behavior-compatible and
off by default. Rollback removes only the optional controller and generated
product fields for that slice.

## Qualification boundary

Software and simulator tests cannot establish real firmware-transfer safety,
IDN interoperability, EoE throughput, physical redundancy timing, real DC
precision, or product-safe recovery. Each capability retains a separate HIL
and release qualification requirement.
