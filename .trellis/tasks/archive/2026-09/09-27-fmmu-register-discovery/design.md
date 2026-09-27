# Design

## Boundaries

The new `fmmu_discovery` core module owns register-page decoding and the
caller-driven read FSM. `startup` owns per-position sequencing and transactional
evidence publication. `mapping_config` owns complete-bank clearing and mapping
readback. Product configuration remains the owner of desired logical addresses;
the discovered pages are evidence and reset bounds, not desired configuration.

## Data Flow

```text
ScanRecord.fmmu_count + station_address
    -> FmmuRegisterDiscoveryController
    -> staged FmmuRegisterBank
    -> SII stream/signature/DC verification
    -> Startup verified_fmmu_registers(position)
    -> Startup::start_mapping_for_position(...)
    -> MappingConfigController::start_with_verified_fmmus(...)
    -> clear/readback every discovered slot
    -> existing SM and desired FMMU write/readback sequence
```

## Register Contracts

`FmmuRegisterDescriptor` stores the exact 16-byte page and exposes decoded
standard fields. Decoding cannot fail once the exact page length is known;
disabled/reset pages with zero length are valid observations. The bank binds
position, station address, reported count, and ordered descriptors.

The discovery action is a fixed-address `Read` of 16 bytes at
`ESC_FMMU_BASE + index * ESC_FMMU_STRIDE`. Every action uses WKC 1 and exact
generation/token/address/operation/length/deadline ownership. The controller
stages descriptors internally and makes `bank()` available only in `Complete`.

## Startup Sequencing

Only profiles with `expected_sii` enter `ReadingFmmuRegisters`. This preserves
legacy traffic shape and ties register evidence to the existing FMMU usage
contract. Identity and optional mailbox verification run first. Descriptor
discovery runs next, then the SII category stream. The discovered bank is held
in a staging slot and copied into position-keyed verified storage only after
the SII signature and optional DC mode both match.

`StartupAction` delegates the new action through the same control-pool surface.
Restart resets the discovery controller, staging slot, and verified banks.

## Mapping Sequencing

The existing `start` method retains its current source-compatible sequence.
The enhanced path stores the discovered count and enters two new phases:

1. write zero page for slot N;
2. read back slot N and require all zero;
3. repeat through the discovered count;
4. run existing SyncManager write/readback;
5. run existing desired FMMU write/readback.

Preflight requires matching station address, discovered count within capacity,
every desired FMMU index below that count, and desired count not exceeding it.
All checks finish before controller mutation or action publication.

## Compatibility

- Existing Startup profiles without expected SII remain unchanged.
- Existing `MappingConfigController::start` remains available.
- Generated product hashes and artifacts do not change because discovery is
  live runtime evidence, not desired product semantics.
- The scheduler sees only additional mapping phases/actions and reuses the
  existing immutable action/control-request matching contract.

## Failure and Rollback

All failures are typed fixed-size enums. A failed discovery publishes no bank;
a failed Startup verification publishes none of the three related evidence
records; a failed clear/readback leaves Mapping faulted and keeps the Startup
configuration barrier closed. Reverting the implementation commit restores the
old optional mapping path without changing generated configuration formats.
