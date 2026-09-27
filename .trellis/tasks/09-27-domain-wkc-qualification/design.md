# Design: Per-Domain cyclic WKC qualification

## Existing flow

`RxIndexTable::validate_and_complete` validates a received datagram and returns
`RxIndexError::WorkingCounterMismatch` after every non-WKC identity check has passed.
`CyclicEngine` currently records only an aggregate cycle mismatch count. Validated
datagrams are delivered through `RxDatagramConsumer::accept`, so a rejected WKC frame
never reaches the Domain and its actual WKC cannot be attributed.

`ScheduledDomainBank` already owns a fixed `[u8; 256]` index-to-Domain table. ProcBuf
ABI v6 already reserves two bytes in each per-Domain quality record.

## Engine observation API

Add a public copyable `RxWorkingCounterMismatch` value containing:

- receive `slot_id`
- receive `generation`
- `expected_wkc`
- `actual_wkc`

Extend `RxDatagramConsumer` with a default no-op
`observe_working_counter_mismatch` method that also receives cycle, receive timestamp,
and the already parsed `DatagramHeader`. Forward this observer through reference and
multiplexing implementations.

The engine invokes the observer only in the
`RxIndexError::WorkingCounterMismatch` branch, using the pre-validation index entry and
received datagram header. No callback is emitted for deadline, generation, address,
size, command, malformed-frame, duplicate, or timeout errors.

## Domain state machine

Add two bounded fields:

- `wkc_mismatch_seen`: transient flag reset by `begin_receive`
- `consecutive_wkc_mismatches`: persistent saturating `u16`

The observer accepts evidence only when the Domain receive window is active, the
generation matches, and the index belongs to a configured segment. It adds the actual
WKC to the cycle diagnostic total using saturating arithmetic and sets the transient
flag. The RX index table has already rejected the slot, so the same datagram cannot
successfully complete or be counted twice.

At `finish_receive`:

- mismatch seen: increment the persistent counter once and reject the staged image;
- no mismatch seen: reset the counter to zero, regardless of success or another failure;
- no `finish_receive` because the Domain was not due: leave the counter unchanged.

The existing `valid`, `complete`, `last_valid_cycle`, and `input_age_cycles` semantics
remain unchanged.

## Scheduled routing

Extend the sealed scheduled-Domain receive interface with a mismatch observation
operation. `ScheduledDomainBank` resolves the owner directly from `index_owner`, then
forwards only to that Domain. The concrete Domain repeats its active/generation/index
checks, keeping the ownership table an optimization rather than the sole safety check.

## ProcBuf and lifecycle

Rename the ABI v6 `reserved: u16` member to
`consecutive_wkc_mismatches: u16`. Although the binary size and offsets remain stable,
the field gains runtime meaning. Per the project ABI contract, bump to ABI v7 and let
the version participate in a new layout hash. Version 6 attachments are rejected so a
zero written by an old producer cannot masquerade as a valid diagnostic observation.

The lifecycle ProcBuf projection copies the counter. Safety qualification continues to
use current-cycle `actual_wkc == expected_wkc`, aggregate receive validity, and existing
guard debounce. The diagnostic counter is not part of a permissive decision.

## Verification strategy

- Unit-test the exact engine callback boundary.
- Unit-test direct Domain episode semantics and payload rejection.
- Unit-test scheduled owner routing across multiple Domains.
- Assert ProcBuf record size/offset stability, ABI v7 descriptor/hash generation, and
  rejection of an ABI v6 header.
- Extend lifecycle tests to prove diagnostics do not alter fail-closed debounce.
- Run the repository's focused tests, `no_std` checks, validators, and full `make ci`.

## Documentation boundary

Mark deterministic software mismatch attribution and per-Domain episode diagnostics as
implemented. Continue to list real-slave full-period WKC, target ports, interoperability,
and HIL evidence as open qualification work.
