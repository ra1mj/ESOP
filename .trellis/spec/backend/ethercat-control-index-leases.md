# EtherCAT Control Wire Index Leases

## 1. Scope / Trigger

Use an explicit reserved wire namespace whenever Startup, SII, configuration,
mailbox, monitoring or diagnostic actions share a master with cyclic Domain/DC
traffic. Logical FSM indices wrap; they are not a valid global wire allocation
strategy. A long SII read can issue more than 1000 actions before OP.

## 2. Signatures

```rust
let mut controls = ControlRequestPool::<8>::new_with_datagram_indices_excluding(
    &[220, 221, 222, 223, 224, 225, 226, 227],
    &[0, 1, 2, 3, 240, 241], // complete frozen Domain + cyclic DC indices
)?;
let logical_index = request.action_datagram_index();
let actual_wire_index = request.datagram_index;
```

`new_with_datagram_indices(indices)` freezes a namespace without an explicit
cyclic exclusion list. The old `new()` remains direct-index compatible.

## 3. Contracts

- Configure 1..64 unique wire indices and a pool capacity of 1..64 before
  activation. The number of indices may differ from the number of slots.
  Every occupied request holds exactly one unique lease until explicit release,
  including prepared, in-flight, complete and failed requests.
- Acquisition validates payload/response bounds and available slot/lease before
  publishing ownership. A bounded rotating search selects a free lease; no heap,
  lock, wait or secondary transport owner exists.
- The request privately retains its immutable logical action index and original
  lease. Its public `datagram_index` is the actual wire index. Frame building,
  RX arming, prior-generation ownership, header completion and schedule conflict
  checks use the wire index. `matches_action` and controller completion compare
  the original logical index, keeping token/generation/address/size/WKC/deadline
  checks unchanged.
- Service code must use `matches_action` or `action_datagram_index()`; comparing
  a service action index directly with `request.datagram_index` is invalid for
  leased pools. An explicit public-wire-field mutation is rejected before frame
  build/completion, and release frees the private original lease.
- Build/TX/RX through the existing pool/master/production owner. Do not construct
  a manual frame with an FSM's logical index after allocating a leased request.
- The caller reserves a namespace disjoint from every frozen Domain/DC plan.
  The exclusion constructor validates that before activation; the production
  scheduler still preflights the actual wire index before every submission.
  Lease rotation is not a replacement for deadlines/generation validation.

## 4. Validation & Error Matrix

| Condition | Result |
| --- | --- |
| Pool capacity zero or above 64 | `TooManyRequests` |
| Empty namespace or more than 64 entries | `InvalidDatagramIndices` |
| Duplicate namespace entry | `DuplicateDatagramIndex` |
| Namespace overlaps supplied cyclic/DC indices | `ReservedDatagramIndex` |
| Free slot exists but every wire lease is retained | `DatagramIndicesExhausted` |
| Every request slot occupied | `SlotBusy` |
| Returned header uses logical rather than leased index | `DatagramIndexMismatch` |
| Public wire field changed after acquisition | Build/completion rejects; release remains correct |

## 5. Good / Base / Bad Cases

- Good: two independent FSMs both emit logical index 1 and receive different
  wire leases while retaining their original immutable action identities.
- Base: legacy `new()` emits the supplied logical index directly.
- Bad: SII logical index 240 wraps into cyclic DC index 240. A direct pool
  causes a real schedule conflict; a leased pool translates it before TX.
- Bad: a terminal failed request remains unreleased. Its lease stays owned;
  another acquisition fails locally instead of aliasing it.

## 6. Tests Required

`crates/esop-ethercat-core/tests/control_index_leases.rs` covers constructor
validation, concurrent identical logical indices, terminal retention/exhaustion,
wire header matching, duplicate response rejection, immutable lease release and
real mailbox completion after translation. Its integration test executes 1024
SII register actions through the production scheduled bank alongside Domain
indices 0..3 and DC 240/241, verifies logical index wrap, all EEPROM words and
1024 successful DC samples without unmatched or rejected control responses.

## 7. Wrong vs Correct

Wrong: every protocol FSM independently increments an eight-bit wire index and
assumes its sequence cannot collide with an active cyclic plan.

Correct: preserve the FSM's logical identity, lease its actual wire index from
one frozen pool namespace, and retain ordinary exact RX ownership validation.
