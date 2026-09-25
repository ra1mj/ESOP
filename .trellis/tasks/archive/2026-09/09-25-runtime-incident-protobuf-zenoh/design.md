# Design: runtime incident Protobuf and Zenoh projection

## Boundaries

`esop-ebpf-agent` remains the fixed-capacity source of truth and keeps no
dependency on Protobuf, Zenoh or allocation. `esop-proto` remains the versioned
wire contract. A new host-only adapter in `esop-zenoh-gateway`, compiled under
the existing `zenoh` feature, validates and projects the agent shape before the
existing transport path serializes it.

```text
RuntimeAgent::pop_incident()
  -> project_runtime_incident(&AgentRuntimeIncident)
  -> ProtoRuntimeIncident (schema v1, additive fields)
  -> ZenohGateway::publish_agent_incident(...)
  -> existing diagnostic route / QoS / observation markers
  -> subscriber or typed query provider
```

No adapter is called from the EtherCAT cycle, MLG decision or ProcBuf writer.

## Protobuf additions

Keep every existing field and reservation. Append machine-readable fields after
the existing schema-version tag.

`RuntimeIncident` gains boot/epoch/confidence, cycle first/last, the configured
evidence window, PID/TID, CPU/IRQ/ifindex, observed value, threshold and event
count.

`RuntimeEvidence` gains evidence ID, agent epoch, transition sequence, evidence
domain/severity, IRQ/ifindex, exact 64-bit observed value, threshold, duration,
count and kind-specific detail. The existing 32-bit `value` is populated with
`min(observed_value, u32::MAX)` for old readers; the additive 64-bit field is
authoritative.

The frozen v1 fixture is intentionally not regenerated. It represents an old
reader and must continue to decode the legacy subset while ignoring additions.

## Mapping contract

- ID: `esop-<boot:016x>-<epoch:016x>-<incident:016x>`.
- Severity: agent Info/Warning/Error/Critical maps to Protobuf numeric values
  1/2/3/4; zero remains unspecified.
- Reason code and evidence kind/domain use the agent's stable `repr(u8)` numeric
  discriminants as unsigned machine codes.
- `cycle_sequence` remains the legacy last cycle; additive fields preserve the
  full first/last range.
- Suggested actions use stable snake-case literals:
  `continue_observe`, `degrade_host_observation`, `controlled_stop`,
  `latch_fault`.
- Affected component is a stable category derived from the existing incident
  code, such as `host.scheduler`, `host.irq`, `host.network`, `host.memory`,
  `host.cpu`, `user.component`, `user.zenoh`, `user.esop.raw_port` or
  `observer.agent`. It does not assert a more specific causal root.
- Legacy `RuntimeEvidence.attach_point` remains zero because the agent evidence
  ABI does not expose a tracepoint/uprobe identifier. The additive domain and
  kind fields carry supported source classification.

## Validation

Projection requires nonzero incident/boot/epoch identities, ordered incident
windows and cycle ranges, and `1..=MAX_INCIDENT_EVIDENCE` records. Every active
evidence record must have a nonzero ID, matching boot/epoch, an inclusive
timestamp inside the incident window and, when the incident cycle range is
nonzero, an inclusive cycle inside that range.

The Protobuf-side query validator applies equivalent checks using the additive
fields. It validates incident IDs and incident boot directly, not only nested
evidence. Incident query ordering remains provider-defined; only state records
participate in `after_sequence` monotonicity.

## Errors and publication

`IncidentAdapterError` enumerates validation failures without heap-allocated
error strings. `RuntimeError::Incident` preserves this category separately
from schema, route and Zenoh transport failures. Projection completes before
the existing publish observation marker begins, so rejected local data is not
reported as a transport attempt.

## Compatibility and rollback

- Default gateway builds retain `no_std` and do not pull agent/proto/Zenoh
  dependencies.
- Existing `publish_incident(&ProtoRuntimeIncident)` remains available.
- Existing wire tags are unchanged; old readers ignore additive fields.
- Rollback removes the adapter and additive fields without changing the agent
  or BPF ABI. Data already emitted with additive fields remains decodable by
  old v1 readers as the legacy subset.
