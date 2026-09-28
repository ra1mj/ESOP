# Design: runtime diagnostics CLI and eBPF status

## Data flow

```text
EtherCAT cycle + DC + lifecycle + eBPF agent
  -> ProcBuf StatePage / RuntimeIncident ring
  -> ProcBufProjector (single validation and projection owner)
  -> additive esop.v1 RobotState.operational + QueryReply.incidents
  -> Zenoh typed query route
  -> esop command parser
  -> transport-independent view selection and text renderer
```

The real-time crates do not depend on the CLI, Protobuf, Zenoh, or Aya. The
host supervisor remains responsible for publishing snapshots and serving the
existing typed query callback.

## Protobuf contract

Append `OperationalStatus operational` to `RobotState`. The nested status owns
raw operational facts rather than duplicating the debounced `QualitySummary`:

- `link_up`, `al_state`, `fault_bitmap`, `command_age_cycles`,
  `deadline_misses`;
- `dc_locked`, `dc_offset_ns`;
- repeated ordered `DomainStatus` records;
- optional `RuntimeObservationStatus` for eBPF agent health and loss.

All additions use new field numbers. The frozen baseline fixture stays
unchanged so compatibility tests continue to model an old reader.

## Query boot binding

`decode_query_payload` validates schema and robot identity first. A zero boot
ID is replaced with the query provider's nonzero current boot ID. Any nonzero
different boot remains `BootMismatch`. The normalized request is passed to the
provider, so existing reply validation still requires one exact boot and does
not need a wildcard path.

## CLI boundaries

Create `crates/esop-cli/` with:

- a library that owns command parsing, view selection, validation, rendering,
  and exit classification;
- a binary named `esop` that opens Zenoh, performs bounded typed queries, and
  invokes the library;
- no dependency from any real-time crate back to the CLI.

Arguments follow the existing workspace's explicit parser style. Common
transport options are `--fleet`, `--robot`, optional `--config`, `--limit`, and
for watch `--interval-ms` plus an optional bounded iteration count useful for
qualification. Read-only boot discovery is the default; `--boot-id` can pin a
known boot and fail on restart.

## Views

- `status` renders identity, sequence, link/AL, aggregate quality, lifecycle,
  DC, Domain summary and eBPF observation health.
- `domain list` renders one line per ordered Domain.
- `dc` renders lock and signed offset and reports unavailable details clearly.
- `lifecycle` renders state, permit epoch/expiry, masks and fault codes.
- `incident list` renders bounded incident identity, severity, reason,
  confidence, cycle range, loss and suggested action.
- `doctor` reduces the same typed snapshot to explicit checks and a healthy or
  degraded exit code; it does not invent remediation or mutate runtime state.
- `watch` reruns one view with the last state sequence as the next cursor.

## Error model

Parsing, route/configuration, transport, remote rejection, decode, contract,
empty reply, and doctor-degraded outcomes remain distinguishable. The binary
prints human-readable errors only at the process boundary. No string errors
enter no_std crates.

## Compatibility and rollback

- Old Protobuf readers ignore the additive operational field.
- Old providers may return `operational = None`; CLI views report unavailable
  instead of treating missing evidence as healthy.
- Query boot discovery changes only a zero read-only request; exact nonzero
  behavior is unchanged.
- Rollback removes the CLI crate and additive projection while leaving the
  existing state, incident, and query routes intact.

## Qualification boundary

Tests prove projection, compatibility, command parsing, rendering, and local
Zenoh query behavior. They do not prove production router security, target
network timing, live BPF attachment, real NIC/slave behavior, WCET, HIL, or
functional safety.
