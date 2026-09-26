# Verify Generated PDO Configuration Readback

## Goal

Close the software portion of PRD FR-004's PDO configuration verification gap:
turn generated product PDO expectations into a deterministic CoE
assignment/mapping startup plan, and require every configuration write to be
read back byte-for-byte before the controller may advance.

The result must make a successful software configuration run meaningful: a
download acknowledgement alone is insufficient, while any missing, truncated,
or changed readback must fail closed with precise evidence.

## Background

- `docs/esop-software-prd.md` FR-004 is P0 and requires configuration-time PDO
  writes to be read-back verifiable.
- `PdoConfigPlan` already emits the standard clear/write/publish sequences for
  PDO mapping and assignment objects.
- `PdoConfigController` currently accepts only the SDO download response and
  immediately advances to the next write; it never uploads the object value.
- `SdoTransfer` already supports expedited and segmented uploads and exposes
  the completed payload, so no second CoE implementation is required.
- Generated `ProductPdoConfig` entries already carry the ESI RxPDO/TxPDO
  object index and SyncManager number, but `esop-product-config` does not yet
  turn them into a per-slave `PdoConfigPlan`.
- The existing generated field name `assignment_index` is retained for artifact
  compatibility. In this contract it denotes the PDO mapping object index
  (for example `0x1600` or `0x1A00`); the assignment object is derived as
  `0x1C10 + sync_manager`.

## Requirements

### R1. Exact write/readback state machine

1. Each `PdoSdoWrite` shall run as an SDO download followed by an SDO upload
   of the same index and subindex.
2. The controller shall advance `operation_index` only after the completed
   upload length and bytes exactly match the planned write.
3. Both download and verification requests shall retain the existing token,
   generation, station-address, per-request deadline, and full-action matching
   protections.
4. Segmented upload responses shall remain supported through the existing
   `SdoTransfer` state machine even though generated mapping values are at most
   four bytes.
5. A readback length or byte mismatch shall latch `Faulted`, preserve the
   current operation index, clear pending work, and expose a fixed-size typed
   error containing useful mismatch evidence.

### R2. Observable configuration step

1. `PdoConfigAction` shall identify whether it is performing the download or
   the verification upload so callers and diagnostics do not have to infer the
   state from raw CoE bytes.
2. Existing action matching shall include that step and reject stale or
   modified actions without advancing configuration.

### R3. Generated per-slave plan construction

1. `StaticProductConfig` shall expose an allocation-free builder that selects
   one configured slave and returns its station address with a complete
   `PdoConfigPlan`.
2. For each first-seen SyncManager, the plan shall clear its assignment count
   before modifying any mapping object, then emit each mapping sequence once,
   and finally publish the assignment entries and count.
3. Mapping entries, SyncManagers, and mapping indexes shall preserve generated
   first-seen order, and assignment objects shall use
   `0x1C10 + sync_manager`.
4. The builder shall reject unknown slaves, PDO/domain ownership mismatch,
   unsupported SyncManager values, inconsistent reuse of one mapping object,
   per-slave fixed-capacity overflow, and plan-capacity overflow.
5. Construction shall be transactional: errors return no partially published
   startup plan.

### R4. Deterministic generated contract

1. The checked-in dual-axis simulator generated Rust module shall continue to
   compile and match cfggen output byte-for-byte.
2. The generated product semantic hash shall not change solely because a new
   runtime consumer is added.
3. Existing generated C/JSON contracts and the public `assignment_index` field
   spelling shall remain compatible in this task.

### R5. Real-time and safety constraints

1. Core and product runtime code shall remain `no_std`, allocation-free,
   bounded, and free of blocking synchronization.
2. All new public fallible paths shall use fixed-size typed errors.
3. No write acknowledgement, generated expectation, or simulator response may
   be described as proof that a physical drive accepted the mapping.

## Acceptance Criteria

- [x] One planned write produces a download action, then an upload verification
      action, and completes only after exact readback.
- [x] A multi-operation assignment/mapping plan preserves operation order and
      requires two transfer phases per write.
- [x] Wrong readback length and wrong readback bytes each fault with typed
      evidence and leave `operation_index` unchanged.
- [x] Segmented verification upload can advance and complete through the
      existing CoE transfer machinery.
- [x] Stale, wrong-generation, timed-out, malformed, or modified verification
      actions fail closed under the existing controller rules.
- [x] Generated product data builds deterministic plans for both simulated
      drives and the IO slave, including exact mapping and assignment object
      write sequences and station addresses.
- [x] Unknown slave, owner/domain mismatch, invalid SyncManager, inconsistent
      mapping metadata, fixed scratch overflow, and insufficient plan capacity
      are covered by rejection tests with no partial result.
- [x] cfggen golden output, generated-module compilation, product activation,
      workspace tests, strict Clippy, release/no-std checks, BPF build, and
      Zenoh tests pass.
- [x] PRD, product configuration, robotics plan, capability claims, and Trellis
      product-configuration guidance distinguish delivered software readback
      from production scheduler integration and physical HIL evidence.

## Out of Scope

- Adding `PdoConfigController` to `ScheduledProductionServiceScheduler` or
  changing production service priorities/budgets.
- Live mailbox transport, physical slave activation, vendor interoperability,
  HIL, or measured timing qualification.
- Complete Access, SDO Information, dynamic object discovery, or full ESI/SII
  runtime parsing.
- Watchdog, SyncManager register, or FMMU register configuration changes; their
  existing controllers remain separate.
- Renaming the generated C/JSON/Rust `assignment_index` field or changing the
  input/output schema version.
