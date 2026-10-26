# EtherCAT Runtime Mailbox and CoE

## 1. Scope / Trigger

Read before changing mailbox transport, SDO requests, PDO configuration,
reconfiguration handoff, Emergency delivery or production service arbitration.

## 2. Signatures

```rust
type SdoTransfer = BoundedSdoTransfer<MAX_SDO_DATA>;
let mut requests = SdoRequestBank::<32, 8, 1024>::new();
requests.bind_verified_slave_with_transport(slot, station, config, token)?;
requests.bind_verified_mapped_status(station, mapped)?;
let services = services.with_runtime_sdo(&mut requests);
let terminal = report.runtime_sdo_terminal();
let lost = report.runtime_sdo_lost_events();
```

Submit upload/download with an absolute deadline. Query `status(handle)` and
`result(handle)`; explicitly `release(handle)` before a slot can be reused.
`PdoConfigBatch::take_transport_state` and bank `take_transport_state` transfer
the non-Copy station-bound token to its next physical owner. Reconfiguration
requires explicit `adopt_mailbox_transport` and returns it terminally.

## 3. Contracts

- The physical mailbox is one authority per slave. Preserve its counters and
  wire identities across configuration/runtime/repair; do not mint another
  owner or reset counters when a logical SDO begins.
- Check outgoing SM ownership, then write exactly the configured mailbox size
  with zero padding. The header length remains the logical protocol length.
- Lost consumed reads use SM Activation Repeat Request, matching PDI Repeat ACK,
  then reread. Preserve other activation bits and the original absolute
  transfer deadline. A named WKC0/1 policy handles an empty mailbox, never a
  general relaxation of control admission.
- TX counter1..7 belongs to its sender. RX can be zero/nonsequential/reused;
  correlate service/object/toggle and exact control action, not TX counter.
  Shared RX rejects duplicate wire completions. Never suppress a new Emergency
  by payload equality or by its counter alone.
- Normal CoE initiate messages carry object bytes after their size field.
  A canonical upload initiate has ten CoE bytes. Segments may exceed seven
  data bytes; short final segments include the padding indicated by unused bits.
  Object storage is independent of physical mailbox capacity; publish only a
  complete, validated object. Standard PDO mapping encodes
  `(index << 16) | (subindex << 8) | bit_length`.
- Same-slave requests keep FIFO logical ownership; complete wire actions rotate
  between slaves. Runtime failures terminate that request and release the
  scheduler; required configuration faults keep their existing closed gate.
- Create runtime actions only after selection. Prepared action deadlines are
  immutable. The production scheduler owns the only control pool/transport.
- Sample mapped mailbox bits after shared Domain RX commits. Pending/stale/bad
  Domain data is unavailable and cannot trigger a direct status fallback.
- Emergency queues and terminal event queues are bounded and non-blocking.
  Every terminal request emits once; overflow is counted. Each production
  report consumes at most one terminal event even if another service ran.

## 4. Validation & Error Matrix

| Condition | Result |
| --- | --- |
| Invalid mailbox ranges/SM indices/capacity | Reject before changing state |
| Foreign station token or already-owned receiver | Reject handoff |
| Full request table | `QueueFull`, no partial submission |
| Announced/incremental object overflow | Typed SDO error, no successful prefix |
| Repeat ACK absent/wrong or overall timeout | Bounded failure at original deadline |
| Invalid/old handle | Reject without accessing reused result |
| Mapped Domain unavailable | Wait within budget; no direct polling fallback |
| Terminal/Emergency queue full | Count loss; communication continues |

## 5. Good / Base / Bad Cases

- Good: eight1KiB objects coexist with Domain/DC/monitor/register traffic;
  one object abort leaves the other requests and lifecycle configuration intact.
- Base: the legacy SDO alias retains122-byte storage and exact existing APIs.
- Bad: compare independent RX/TX counters, reread an emptied mailbox without
  Repeat ACK, or stamp a1ms wire deadline during an unselected scheduler cycle.

## 6. Tests Required

Run core CoE/mailbox/PDO/bank/reconfigure tests and simulator production SDO
tests. Cover full SM writes, consumed-read loss, ACK loss, zero/reused/wrapping
counters, initial/segmented data, object bounds, mapped idle Emergency,
same-slave FIFO, continuous diagnostics, delayed ownership and transport handoff.
Queued expirations must appear as distinct typed terminal events with loss counts.

## 7. Wrong vs Correct

Wrong: adjust virtual WKC to a generated expectation, or clear a drive's fault
because transport returned. Correct: independent ESC access determines WKC;
communication success grants neither configuration readiness nor motion permits.
