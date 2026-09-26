# Design: ESM timeouts and OpOnly SyncManager safety

## 1. Configuration model

Add a public fixed-size `AlTransitionTimeouts` value containing nanosecond deadlines for
`preop`, `safeop_to_op`, `back_to_init`, and `back_to_safeop`. Define a named
`ETG1020_DEFAULT_TRANSITION_TIMEOUTS_V1` constant so a future ETG revision is an
intentional source change rather than an untracked numeric change.

`StartupConfig.transition_timeout_ns` remains source-compatible. A non-zero value is a
uniform explicit override; zero means use the per-slave transition profile. Existing
constructors switch to zero so newly generated product configurations use the profile,
while callers that already set the field retain their requested behavior.

`StartupSlaveProfile` contains configured slave position, `AlTransitionTimeouts`, and a
16-bit `op_only_output_sync_manager_mask`. The strict product startup entry point
receives one profile per expected slave and validates count, ordered position, and mask
bounds before entering the startup state machine. The legacy entry point synthesizes
default profiles.

## 2. Transition timeout selection

Timeout selection is based on the current observed AL state and requested next AL state:

- a request to INIT uses `back_to_init`;
- SAFEOP to OP uses `safeop_to_op`;
- OP to SAFEOP, or another backward step whose target is SAFEOP, uses
  `back_to_safeop`;
- establishment of PREOP uses `preop`;
- forward PREOP to SAFEOP uses `preop` because the ESI schema has no separate field and
  the PREOP-class budget covers the configuration/establishment path.

The selected duration is converted once into an absolute step deadline. OpOnly writes,
readbacks, AL request, and AL state polling within that transition consume the same
deadline. No sleep or blocking wait is introduced.

## 3. OpOnly protocol controller

Add an allocation-free `OpOnlySyncManagerController` in the core crate. It operates on a
single slave and a 16-bit mask, one SyncManager at a time, using caller-supplied EtherCAT
actions:

1. Write the SyncManager activation byte at
   `ESC_SYNC_MANAGER_BASE + index * ESC_SYNC_MANAGER_STRIDE + 6`.
2. Require exact WKC, generation, completion length, and deadline validity.
3. Read the same activation byte back.
4. Verify activation bit 0 equals the requested enabled/disabled state.
5. Advance to the next set mask bit until complete.

The controller preserves all unrelated activation flags when enabling. Generated
profiles therefore carry the activation-byte template for each OpOnly output rather
than only a mask. The write path must never accidentally clear the OpOnly flag itself.

Diagnostics identify slave position, SyncManager index, requested state, and protocol
failure. OpOnly failures are startup faults, but they do not overwrite an earlier
captured AL status code.

## 4. Startup sequencing

Each slave tracks `Unknown`, `DisabledVerified`, or `EnabledVerified` OpOnly gate state.

For an OpOnly profile in a non-OP observed state:

1. If not already `DisabledVerified`, run disable plus readback verification.
2. Only then issue or continue the AL state transition.

For transition from OP to a lower state:

1. Disable and verify before the AL state request.
2. Issue the AL request and poll using the same step deadline.

For transition into OP:

1. Ensure the OpOnly outputs are disabled while outside OP.
2. Request and observe OP.
3. Enable and readback-verify the declared output SyncManagers.
4. Report the slave complete only after `EnabledVerified`.

For a slave already in OP at startup, enable/readback verification is required before it
can be reported complete. A slave without OpOnly SMs takes the current path without
additional bus traffic.

Add `StartupAction::OpOnly` so the existing executor remains caller-driven. Startup owns
the OpOnly sub-controller and routes completions by phase; it does not add a second
transport abstraction.

## 5. ESI and SII interpretation

### ESI

Extend `EsiDevice` with parsed state-machine timeout profile and ordered SyncManager
descriptors sufficient to compute output direction, activation-byte templates, and the
OpOnly mask. Parse `<Sm>` elements in device order. Treat `Outputs` as RxPDO/output and
`Inputs` as TxPDO/input. `OpOnly=true` is valid only for output entries. Parse timeout
text as positive decimal milliseconds and convert with checked arithmetic.

### SII

Keep the raw activation byte in `SiiSyncManager` for compatibility. Add helpers
`is_enabled()` for bit 0 and `is_op_only()` for bit 3.
`SiiConfigurationCandidate` marks OpOnly output indexes and preserves activation-byte
templates in `MappingTable`. PDO direction and assigned SyncManager information must
agree before accepting the candidate.

## 6. Mapping configuration

`MappingTable` gains a bounded OpOnly output mask and activation-byte templates plus
methods to mark/query them. This avoids changing every existing `SyncManagerConfig`
literal. Validation rejects indexes outside the supported 16 SyncManagers or masks that
refer to missing/non-output descriptors.

During PREOP configuration, mapping writes use a derived activation byte with enable bit
cleared for marked OpOnly outputs. Readback compares against that derived byte. The
stored configured descriptor remains unchanged so it can later provide the activation
flags needed for OP enable.

## 7. Generated product path

The cfggen layer includes both values in `GeneratedSlave`, inventory JSON, generated
Rust literals, and semantic identity input. `ProductSlaveConfig` stores the runtime
profile and OpOnly activation templates. Product validation checks timeout values,
index bounds, RxPDO/output direction, and profile position.

Provide a small bridge from `StaticProductConfig` to the strict startup entry point using
fixed arrays/caller-owned buffers. No heap is added to runtime crates.

## 8. Compatibility and failure boundaries

- Existing `ExpectedSlave` remains unchanged.
- Existing `StartupController::start` remains available and uses default profiles plus
  the explicit uniform override when configured.
- Existing ESI files without timeout/SM metadata remain valid and receive the versioned
  defaults with no OpOnly descriptors.
- Protocol violations fail generation/configuration early where possible; runtime
  transport/readback failures fail startup closed.
- Software tests demonstrate deterministic sequencing, not real-device behavior.

## 9. Verification strategy

- Unit tests: timeout selection, overflow/invalid ESI values, SII bit helpers, mapping
  OpOnly validation, and OpOnly action/completion sequencing.
- Startup tests: all transition classes, override precedence, enter/leave/already-OP
  paths, timeout sharing, WKC/generation/readback failures, and no-Ready guarantee.
- Cfggen/product tests: explicit/default values, generated fixture changes, semantic
  hash changes, and strict profile propagation.
- Integration gates: formatting, clippy, full tests, `no_std`, BPF, simulated HIL, and
  Zenoh checks already defined by project CI.
