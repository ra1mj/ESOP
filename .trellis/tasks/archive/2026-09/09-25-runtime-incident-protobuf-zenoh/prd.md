# Project runtime incidents into Protobuf and Zenoh

## Goal

Close the PRD FR-051 cross-layer gap between the fixed-capacity
`esop-ebpf-agent::RuntimeIncident` produced by the observer and the versioned
`esop.v1.RuntimeIncident` published and queried by the Linux supervision
domain. The adapter must preserve machine-readable incident identity, full
evidence values, boot/epoch provenance, time/cycle windows, severity and
recommended action without placing Protobuf or Zenoh in the RT path.

## Background

The agent already correlates bounded kernel/user evidence into an incident
with a boot-local numeric ID, agent epoch, confidence, exact window, cycle
range, component identity, observed value, threshold, count, loss and up to
eight fixed evidence records. Protobuf v1 and `ZenohGateway` already expose a
diagnostic route, but current tests hand-build a reduced Protobuf incident;
there is no production projection from the agent type and no router test that
publishes a real correlated incident shape.

The existing Protobuf fields are insufficient for a lossless projection:
`RuntimeEvidence.value` is only 32 bits and neither message carries the full
agent epoch and evidence metadata. Protobuf v1 permits additive compatible
fields while frozen old readers ignore unknown fields. Existing tags and
reserved numbers/names must remain unchanged.

## Requirements

- Add only backward-compatible fields to `esop.v1.RuntimeIncident` and
  `esop.v1.RuntimeEvidence`; do not renumber/reuse any existing or reserved tag.
- Preserve incident boot ID, agent epoch, confidence, first/last cycle,
  configured evidence window,
  component PID/TID/CPU/IRQ/ifindex, full 64-bit observed value and threshold,
  and event count. Preserve evidence ID, epoch, transition sequence, domain,
  severity, IRQ/ifindex, full observed value, threshold, duration, count and
  detail.
- Keep legacy fields meaningful: use the incident's last cycle for the legacy
  single cycle field, keep the existing reason/severity/window/action fields,
  and saturate the legacy 32-bit evidence value while retaining the exact
  64-bit value in its additive field.
- Generate a deterministic globally disambiguated string incident ID from the
  boot ID, agent epoch and agent-local incident ID. Restarting an agent must not
  collide with an incident from an earlier epoch.
- Map severity, incident reason, recommended action and affected component to
  stable values. Do not serialize Rust debug strings or infer root cause beyond
  the agent's existing incident code.
- Reject malformed agent incidents before serialization: zero identity,
  inverted windows/ranges, empty or excessive evidence count, zero evidence
  IDs, boot/epoch mismatch, evidence outside the incident time window, or
  evidence outside a nonzero incident cycle range.
- Keep the pure projection in the host/supervision boundary. The default
  `esop-zenoh-gateway` build must remain `no_std`; enabling its `zenoh` feature
  may opt into the optional eBPF agent dependency.
- Add a production `ZenohGateway` method that projects and publishes an agent
  incident through the existing diagnostic route and reports projection errors
  distinctly from route/schema/transport errors.
- Tighten typed query reply validation so a Protobuf incident and all of its
  evidence match the query boot, share one agent epoch, have nonempty stable
  IDs, and satisfy the same window/cycle ordering contract.
- Add unit tests for exact projection, large-value preservation and every
  rejection class; extend old/current Protobuf reader-writer tests to prove
  additive compatibility.
- Replace the hand-built incident in the live Zenoh test with a projected agent
  incident, then decode it after router delivery and verify identity, evidence,
  action, full values and schema version.
- Update the software PRD, Protobuf and Zenoh design docs, capability manifest
  and backend quality guidance with the implemented boundary and remaining
  limitations.

## Acceptance Criteria

- [x] A valid multi-evidence agent incident projects deterministically into a
      Protobuf v1 incident with no loss of any required 64-bit or provenance
      field; the legacy 32-bit value saturates rather than wraps.
- [x] Invalid identity, window, evidence count, evidence ID, boot/epoch,
      timestamp or cycle-range inputs are rejected before encoding or publish.
- [x] The generated incident ID differs across boot IDs, agent epochs and
      agent-local IDs while remaining stable for repeated projection.
- [x] Frozen v1 readers still decode the legacy subset of messages emitted by
      the current writer; current readers see every new additive field.
- [x] Query replies reject cross-boot/cross-epoch or malformed incident evidence
      and accept a projected incident without confusing incident paging with
      `RobotState.sequence` paging.
- [x] A live loopback Zenoh router receives and decodes an incident published
      through the new agent-facing gateway method with exact projected values.
- [x] Default/no-feature and `zenoh` feature builds, workspace tests, Clippy,
      Protobuf validation, loopback Zenoh tests, capability validation,
      formatting and complete `make ci` pass.
- [x] Changes are committed and pushed to `ra1mj/ESOP`; the final GitHub
      Actions run succeeds before the task is archived.

## Out Of Scope

- UI rendering, persistent incident storage, retention, remote upload and
  pagination policy implemented by a query provider.
- Cryptographic identity, remote ACL/certificate lifecycle and production
  Zenoh topology qualification.
- New eBPF evidence kinds, changes to the fixed agent/BPF ABI, direct MLG
  control, or any claim that an inferred incident is a certified root cause.
