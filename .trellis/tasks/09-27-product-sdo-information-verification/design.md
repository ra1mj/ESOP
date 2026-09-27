# Design: Product-controlled SDO Information verification

## Boundary And Ownership

The core owns protocol decoding and deterministic verification. Cfggen owns
the ESI/product trust-boundary join and derives immutable expectations from
selected PDOs. Product runtime revalidates generated data before exposing it.

```text
ESI CoE@SdoInfo + selected PDO entries + product authorization
    -> cfggen capability check and expectation derivation
    -> generated artifacts and semantic/config identities
    -> product runtime anti-tamper validation
    -> SdoInformationPolicy + fixed expectation plan
    -> mailbox-scheduled SDO Information verifier
```

The verifier is a control-plane service. It does not run in the cyclic PDO
path and does not alter mappings based on observed metadata.

## Core Protocol Contract

Add `sdo_information.rs` beside `coe.rs`. It reuses `CoeHeader` and
`CoeService::SdoInformation` and owns:

- `SdoInformationPolicy`, disabled by default;
- typed request/response opcodes and OD-list types;
- `CanopenDataType`, `SdoInfoObjectCode`, `SdoInfoValueInfo`, and
  `SdoInfoObjectAccess`;
- a fixed-capacity `SdoInformationTransfer<const MAX_OBJECTS: usize>`;
- typed object/entry descriptions and progress/error enums.

The six-byte common payload header is CoE header, opcode with the incomplete
bit, one reserved byte, and little-endian fragments-left. Transfer admission
validates policy before any state mutation. Response handling validates the
service and expected response opcode, then consumes at most a fixed fragment
limit. A nonzero fragments-left sequence must decrement exactly. Unknown-count
incomplete responses remain bounded by the same fragment limit.

OD-list responses retain indices up to `MAX_OBJECTS`. Object and entry
responses retain only their fixed metadata; optional name/value data may be
discarded after bounded fragment consumption. This prevents a device-supplied
description string from determining target memory use.

## Verification Contract

`SdoInformationExpectation` contains:

```rust
pub struct SdoInformationExpectation {
    pub slave_position: u16,
    pub index: u16,
    pub subindex: u8,
    pub data_type: CanopenDataType,
    pub bit_length: u16,
    pub required_access: SdoInformationRequiredAccess,
}
```

The verifier accepts a borrowed immutable expectation slice and validates all
entries before starting. The generated order is `(index, subindex)` with
duplicates merged by OR-ing required access. For each new object index it asks
for an object description and checks the returned index and maximum subindex.
It then asks for access-only entry descriptions and checks exact data type and
bit length, required SDO rights, and required RxPDO/TxPDO mapping flags.

Results are transactional: the verified count is internal until every entry
matches. A mismatch moves the verifier to a terminal fault and publishes no
successful plan. The caller owns mailbox retries, timeout, working-counter
validation, and cycle byte/datagram budgets through the existing mailbox and
master layers.

## ESI And Product Contract

Only a direct `Device/Mailbox/CoE` element contributes capabilities:

```xml
<CoE SdoInfo="true" CompleteAccess="true"/>
```

Absent means unsupported. Malformed values fail ESI parsing. `EsiEntry` stores
the exact typed CANopen data type in addition to its derived signedness.

The strict product field is:

```json
"coe": {
  "complete_access": false,
  "sdo_information": true
}
```

The two feature flags are independent. Enabling SDO Information requires
ESI support but does not require Complete Access. Cfggen derives expectations
only for enabled slaves from the product-selected RxPDO/TxPDO entries.

## Generated Data Flow

Each generated slave carries separate `supported` and `enabled` booleans plus
an offset/count into one immutable expectation table. Entries contain slave
position, index, subindex, CANopen data type, bit length, and required access.
The same logical records are emitted to normalized product JSON, inventory,
robot build input, C, and Rust.

ESI semantic identity includes `SdoInfo` support and exact PDO entry data
types. Product configuration identity includes enablement and the derived
expectation records. Changing any of these facts invalidates stale artifacts.

Robot build input uses a dedicated `devices.coe_sdo_information` array rather
than overloading the existing Complete Access records. Its strict validator
checks declared-slave coverage, unique names/positions, `enabled => supported`,
ordered unique expectations, type/width/access ranges, and disabled-plan
emptiness.

## Product Runtime

`ProductSlaveConfig` stores the two feature booleans and its expectation slice.
The static product API provides per-position policy/plan lookup only after
validating:

- `enabled => supported`;
- disabled slaves have no expectations;
- enabled slaves have at least one expectation;
- each expectation belongs to that slave, is ordered/unique, has nonzero
  index/bit length, and requests at least one supported access direction.

Activation runs this validation before publishing topology/configuration
state. Direct mutation of generated Rust data therefore cannot silently enable
the service or substitute a different object plan.

## Compatibility And Rollback

- Existing products omit `sdo_information` and remain disabled.
- Existing SDO/Complete Access APIs and PDO configuration remain unchanged.
- No cyclic layout or ProcBuf ABI changes are required.
- Reverting the optional manifest field and generated records removes the new
  control-plane path without changing process-image layout or mailbox memory.

## Verification Strategy

- Core unit tests cover exact bytes, fragmentation, typed metadata, and every
  verifier mismatch.
- Cfggen tests cover ESI/product parsing, deterministic derivation, hashes,
  strict robot input, and generated C/Rust/JSON.
- Product tests cover lookup and activation anti-tamper rejection.
- A public core integration test routes a request through mailbox/control/master
  and completes a small verification plan with simulated responses.
- Repository-wide generated, `no_std`, lint, and CI gates remain authoritative.
