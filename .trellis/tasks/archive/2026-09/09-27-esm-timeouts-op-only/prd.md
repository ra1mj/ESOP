# Implement ESM timeouts and OpOnly SyncManager safety

## Goal

Close the remaining software scope of EtherCAT master requirement `CFG-009` by making
EtherCAT State Machine (ESM) transition deadlines and `OpOnly` output SyncManager
isolation explicit, generated, deterministic, and fail-closed across ESI, SII, product
configuration, and startup control.

This task provides software evidence only. It does not claim physical HIL, device
interoperability, conformance, or safety certification.

## Requirements

### ESM transition timeouts

- Represent the four transition timeout classes required by the ESI state-machine
  schema: PREOP establishment, SAFEOP to OP, transitions back to INIT, and transitions
  back to SAFEOP.
- Parse per-device timeout values from ESI when present.
- Use a named, version-managed ETG.1020 default profile when ESI/SII does not provide a
  value. The initial profile is `PreopTimeout=3000 ms`,
  `SafeopOpTimeout=10000 ms`, `BackToInitTimeout=5000 ms`, and
  `BackToSafeopTimeout=200 ms`.
- Preserve the existing uniform startup timeout as an explicit compatibility override.
  An override takes precedence over generated/default per-transition values; absence of
  an override selects the profile.
- Select one absolute deadline for each transition step. Any OpOnly preparation and AL
  request/readback belonging to that step share the same deadline.
- Reject zero, malformed, or overflowing timeout values during generation rather than
  silently replacing them.

### OpOnly output SyncManagers

- Parse the ESI `Sm OpOnly="true"` declaration and the SII SyncManager activation
  byte's `OpOnly` flag independently from its enable bit.
- Only output/RxPDO SyncManagers may be declared `OpOnly`; reject contradictory ESI or
  SII configuration.
- Keep every `OpOnly` output SyncManager disabled and readback-verified whenever the
  corresponding slave is not Operational.
- Disable and verify `OpOnly` output SyncManagers before requesting a transition away
  from Operational.
- On transition to Operational, wait until the AL state is observed as Operational,
  then enable and readback-verify the declared `OpOnly` output SyncManagers.
- Do not report startup `Ready` or release process-output lifecycle gates until the
  required SyncManager state is verified.
- A write/readback, WKC, generation, length, or deadline failure must fault startup and
  must not produce `Ready`.
- PREOP mapping configuration must program declared `OpOnly` output SyncManagers with
  activation disabled even when their SII/ESI initial-enable flag is set.

### Configuration propagation

- Carry the timeout profile and OpOnly output SyncManager mask through generated
  inventory, generated Rust product configuration, semantic hashes, and runtime product
  structures.
- Expose a product-configured startup path that supplies an exact profile for every
  configured slave.
- Validate profile count, slave position, mask bounds, and direction consistency before
  startup work is emitted.
- Keep existing caller-owned buffers, fixed capacities, `no_std`, and allocation-free
  runtime properties.
- Preserve the existing legacy `StartupController::start` path and existing
  `ExpectedSlave` representation.

### Evidence and documentation

- Add focused unit and integration tests for explicit ESI values, default fallback,
  malformed input, SII bit interpretation, per-transition deadline selection, complete
  OpOnly enable/disable sequencing, readback failure, and product propagation.
- Update requirements/capability documentation with implemented software evidence and
  keep physical HIL/interop evidence explicitly open.
- Keep mailbox configuration discovery and SM/FMMU/DC descriptor auto-generation out of
  this task; they remain separate roadmap items.

## Acceptance Criteria

- [x] ESI parsing accepts explicit state-machine timeouts and `OpOnly` output SMs, uses
      the versioned defaults when values are absent, and rejects invalid declarations.
- [x] SII parsing treats activation bit 0 as enable and bit 3 as `OpOnly`, and rejects
      `OpOnly` declarations that are not associated with output/RxPDO mapping.
- [x] Generated JSON/Rust/product configuration and semantic/configuration hashes include
      the exact per-slave timeout profile and OpOnly mask.
- [x] Startup selects the correct timeout for each AL transition and retains the uniform
      compatibility override with documented precedence.
- [x] In non-OP states, declared OpOnly output SMs are disabled and verified before the
      relevant AL transition or readiness decision.
- [x] Entering OP enables and verifies OpOnly output SMs only after OP is observed;
      leaving OP disables and verifies them before the AL request.
- [x] Any OpOnly protocol/transport/deadline/readback failure prevents `Ready` and
      produces deterministic diagnostics without overwriting the first AL fault record.
- [x] Mapping configuration writes OpOnly output SMs disabled during PREOP and validates
      readback accordingly.
- [x] Legacy startup callers compile and retain explicit uniform-timeout behavior.
- [x] Focused tests, workspace formatting/lint/type/test gates, `no_std`, BPF, simulated
      HIL, and Zenoh integration checks pass locally.
- [x] Requirement and capability documents distinguish software completion from pending
      physical HIL, interoperability, and qualification evidence.

## Out Of Scope

- Physical EtherCAT hardware execution or conformance-lab evidence.
- Automatic MailboxConfig, complete SM/FMMU descriptor, or DC descriptor discovery.
- Safety certification, SIL claims, or production-release qualification.
- Changing the process-data lifecycle policy beyond the new startup readiness gate.
