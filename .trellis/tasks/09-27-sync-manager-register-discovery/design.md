# Design

## Boundaries

The new `sync_manager_discovery` core module owns register-page decoding and the
caller-driven read FSM. `startup` owns per-position sequencing and transactional
evidence publication. `mapping_config` owns complete-bank clearing and desired
mapping readback. Product configuration remains the owner of intended
SyncManager fields; discovered pages are evidence and reset bounds only.

## Data Flow

```text
ScanRecord.sync_manager_count + station_address
    -> SyncManagerRegisterDiscoveryController
    -> staged SyncManagerRegisterBank
    -> SII stream/signature/DC verification
    -> Startup verified_sync_manager_registers(position)
    -> Startup::start_mapping_for_position(...)
    -> MappingConfigController::start_with_verified_registers(...)
    -> clear/readback every discovered SyncManager slot
    -> clear/readback every discovered FMMU slot
    -> existing desired SM/FMMU write/readback sequence
```

## Register Contracts

`SyncManagerRegisterDescriptor` stores the exact eight-byte page and exposes
little-endian physical start/length plus control, status, activation, and PDI
control bytes. Exact length is established by the action contract, so zeroed and
disabled pages decode without a fallible parser. The bank binds position,
station address, reported count, and ordered descriptors.

Each discovery action is a fixed-address `Read` of eight bytes at
`ESC_SYNC_MANAGER_BASE + index * ESC_SYNC_MANAGER_STRIDE`. Every action requires
WKC 1 and exact generation/address/operation/length/deadline ownership. The
controller stages descriptors internally and exposes `bank()` only in
`Complete`.

## Startup Sequencing

Only profiles with `expected_sii` enter register discovery. Identity and
optional mailbox verification run first, followed by FMMU discovery,
SyncManager discovery, and then the SII category stream. The two banks are held
in staging slots and copied into position-keyed verified storage only after the
SII signature and optional DC mode both match.

`StartupAction` delegates both register action types through the same control
pool surface. Restart and any terminal failure reset both discovery controllers,
staging slots, and verified arrays.

## Mapping Sequencing

The existing legacy and FMMU-only starts retain their behavior. A new full
verified-register path stores both discovered counts and runs:

1. zero-write SyncManager slot N;
2. exact zero readback for SyncManager slot N;
3. repeat through the discovered SyncManager count;
4. run existing FMMU bank clear/readback;
5. run existing desired SyncManager write/readback;
6. run existing desired FMMU write/readback.

Preflight validates bank position/station/count, desired counts, and every
configured index before controller mutation. The Startup bridge supplies both
banks from one position-scoped verified transaction.

## Compatibility

- Startup profiles without expected SII remain unchanged.
- `MappingConfigController::start` remains unchanged.
- `start_with_verified_fmmus` remains available and keeps its current order.
- Generated product hashes and artifacts do not change because live discovery
  is not product semantics.
- Production scheduling sees additional mapping phases/actions but no new
  request ownership model.

## Failure and Rollback

All failures use fixed-size typed enums. A failed discovery exposes no bank; a
failed Startup transaction publishes none of the related evidence; a failed
clear/readback leaves Mapping faulted and the PREOP configuration barrier
closed. Accepted ESC writes are never described as rolled back. Reverting the
implementation commit restores the previous optional FMMU-qualified path
without changing generated formats.
