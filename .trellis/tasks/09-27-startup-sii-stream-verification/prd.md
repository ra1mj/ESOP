# Startup-owned SII stream verification

## Goal

Close the remaining CFG-007 software gap by making `StartupController` acquire
and verify each expected slave's complete bounded SII SyncManager/RxPDO/TxPDO
category stream before that slave emits its first AL transition action.

Generated product configuration must provide the expected structural
SyncManager and ordered PDO topology. A live stream fault, projection fault,
capacity fault, or topology mismatch must latch Startup faulted, publish no
verification evidence, and prevent PREOP/SAFEOP/OP progress.

## Background

- `SiiCategoryStreamReader` and `SiiStreamDiscoveryController` already provide
  allocation-free acquisition from standard word `0x0040` through END, one
  absolute deadline, first-fault retention, and atomic
  `SiiConfigurationCandidate` publication.
- Startup currently verifies identity and optional standard CoE mailbox words,
  then begins AL transition without consuming the category candidate.
- `esop-cfggen` already resolves ESI SyncManagers and selected ordered PDOs,
  while `StaticProductConfig::startup_profiles()` already supplies generated
  timeout, OpOnly, and mailbox expectations to Startup.
- Product ESI does not always declare physical start/length/control fields for
  process-data SyncManagers. Exact cross-source comparison is therefore scoped
  to structural SM facts available in both sources (count, enabled mask,
  OpOnly mask) and the complete selected ordered PDO topology. Live physical SM
  ranges remain validated internally by `SiiConfigurationCandidate`; mailbox
  physical ranges remain covered by the existing standard-header check.

## Requirements

### R1. Versioned topology signature

1. Core shall expose a fixed-size, versioned SII configuration signature and a
   transactional builder usable from `no_std` code without allocation.
2. The canonical input shall include exact SyncManager count, enabled mask,
   OpOnly mask, Rx/Tx PDO counts, entry counts, and for every PDO in frozen
   direction/order: PDO index, SyncManager index, object index, subindex, and
   bit length.
3. The candidate projection shall retain each PDO index and exact entry range
   so the live signature is derived from parsed evidence rather than inferred
   from aggregate lengths.
4. Signedness shall remain a generated product semantic and shall not be part
   of the live signature until the SII PDO flags have a separately specified
   data-type contract.

### R2. Generated expectation

1. `esop-cfggen` shall retain each generated slave's ESI SyncManager count and
   enabled mask in normalized JSON, C, and Rust artifacts.
2. `StaticProductConfig::startup_profiles()` shall validate those bounds and
   build the expected signature from the slave's structural SM facts,
   `OpOnlySyncManagerProfile`, and selected PDO entries before mutating Startup.
3. Invalid SM masks, PDO grouping, capacities, directions, or empty mappings
   shall return a typed `ProductStartupError` and publish no profile array.
4. Generated semantic/config hashes shall continue to cover these ESI facts.

### R3. Startup ownership and ordering

1. A profile with an expected SII signature shall cause Startup to enter a
   distinct SII configuration-reading phase after identity and optional mailbox
   verification, but before `TransitioningAl`.
2. Startup shall own one fixed-capacity stream/discovery workspace reused in
   slave order. Acquisition shall begin at standard SII word `0x0040` and use a
   dedicated absolute configuration timeout plus the existing per-request
   timeout.
3. `StartupAction` shall identify configuration-stream actions separately from
   identity and mailbox actions so stale or cross-routed responses fail closed.
4. Projection and comparison shall occur atomically after END. Only an exact
   signature match may publish position-keyed verified evidence and begin AL.
5. The existing `start()` API and profiles without an expected signature shall
   preserve the legacy identity/mailbox-to-AL path.

### R4. Fault and evidence semantics

1. Stream, projection, signature-construction, mismatch, WKC, generation,
   payload, action ownership, capacity, and deadline failures shall be typed
   Startup faults.
2. No mismatch or partial stream may populate verified evidence or emit an AL
   action. Restart shall clear old stream state and evidence.
3. The first terminal stream/discovery error shall remain stable after later
   polling or timeout calls.
4. Existing verified mailbox evidence may remain diagnostic evidence, but it
   must not imply SII configuration verification or permit AL progress.

### R5. Production and documentation integration

1. Existing production scheduling shall route the new Startup action through
   the same single-request ownership checks without introducing a separate
   scheduler service or unbounded work.
2. README, PRD/requirements, product configuration contract, capability
   manifest, and generated example artifacts shall describe the completed
   software boundary and preserve explicit physical/HIL limitations.
3. Core and product configuration shall remain `no_std`; the activated cyclic
   path shall gain no heap allocation, blocking, sleeping, or text logging.

## Acceptance Criteria

- [x] Exact generated/live SM and ordered PDO evidence verifies each slave and
      no AL action is observable before verification completes.
- [x] SM count/enabled/OpOnly mismatch and PDO index/SM/object/subindex/bit
      length/order/count mismatch each fault Startup with no verified evidence.
- [x] Missing END, capacity exhaustion, malformed category, bad WKC,
      generation mismatch, stale action, and absolute timeout all remain
      fail-closed and preserve the first terminal error.
- [x] Multi-slave startup reuses the bounded workspace in expected slave order
      and retains position-keyed signatures only for completed matches.
- [x] Legacy profiles without SII expectations continue to reach the existing
      AL path unchanged.
- [x] Generated simulator artifacts are deterministic, compile as C/Rust, and
      their startup profiles carry exact expected signatures.
- [x] Focused core, product-config, cfggen, and scheduler/simulator regressions
      pass, followed by the repository `make ci` quality gate.

## Out Of Scope

- FMMU or DC category semantic discovery and automatic per-slave configuration.
- Authenticating that caller-delivered EEPROM responses originated from the
  intended physical slave.
- General ESI catalogs with alternative/default PDO sets that differ from the
  selected product mapping; the supported subset requires exact selected/live
  topology equality.
- Decoding SII PDO flags into signedness or vendor-specific data types.
- Target WCET/resource qualification, interoperability certification, or
  physical HIL evidence.

## Notes

- This is a software evidence boundary. It must not upgrade any physical or
  functional-safety qualification claim.
