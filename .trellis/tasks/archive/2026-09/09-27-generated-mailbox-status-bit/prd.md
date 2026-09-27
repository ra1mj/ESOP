# Generated mailbox status-bit discovery

## Goal

Complete the software-only portion of FR-016 / MBX-004 for direct
SyncManager input-mailbox status polling. Product configuration generated from
ESI shall carry a canonical mailbox-full Status Bit policy into the no_std
runtime, and Startup shall cross-check that policy against online SII evidence
before the slave is allowed to enter OP.

## Requirements

1. Preserve the MBoxOut and MBoxIn SyncManager indices discovered from ESI.
2. Treat an ordered `MBoxState` / SyncManager-status FMMU usage as the product
   declaration that Status Bit polling is supported. Products without that
   declaration shall retain the existing PollTime behavior.
3. Derive the direct Status Bit policy from the MBoxIn SyncManager index using
   the standard ESC SyncManager status-byte address, mailbox-full mask `0x08`,
   and active-high polarity. Do not accept arbitrary generated values.
4. Emit the discovered indices and policy through generated JSON, inventory,
   C, and Rust artifacts. Generated Rust product configuration shall construct
   a status-enabled `MailboxConfig` without caller-side patching.
5. Revalidate the generated policy when building product startup profiles and
   PDO batches. Tampered or internally inconsistent generated configuration
   shall fail closed.
6. During Startup, cross-check the expected MBoxIn SyncManager index against
   the online SII SyncManager category, including mailbox address, size,
   control byte, and enabled state. Existing ordered FMMU-usage verification
   shall continue to validate the Status Bit capability declaration.
7. Publish the verified mailbox configuration only after the SII checks pass.
   The production scheduler shall then use the existing mailbox controller to
   read the status byte before issuing an input-mailbox read.
8. Preserve public compatibility for manually configured
   `MailboxConfig::with_status_bit` users and for products that use PollTime.
9. Keep this increment deterministic and allocation-free in no_std runtime
   crates.

## Out Of Scope

- Mapping the mailbox status bit through an FMMU into cyclic process data.
- Emergency-message semantics beyond the mailbox transport behavior already
  implemented.
- Hardware timing, physical-frame, HIL, or WCET claims.

## Acceptance Criteria

- [x] ESI parsing retains mailbox SyncManager indices and rejects incomplete or
      contradictory Status Bit declarations.
- [x] Generated JSON, inventory, C, and Rust outputs expose a deterministic
      direct Status Bit policy when the ESI declares SyncManager status usage.
- [x] Products without the declaration generate the existing PollTime policy.
- [x] Product configuration rejects a non-canonical address, mask, polarity,
      or mailbox SyncManager index before startup/runtime construction.
- [x] Startup rejects online SII with a missing, disabled, or mismatched MBoxIn
      SyncManager descriptor before AL transition.
- [x] A production-scheduler test proves an inactive status byte suppresses the
      input-mailbox read and an active status byte permits it.
- [x] Existing manual Status Bit and PollTime tests remain green.
- [x] Relevant focused tests, no_std checks, clippy, and full `make ci` pass.
- [x] Product and compliance documentation distinguish completed direct
      SyncManager-register discovery from the remaining FMMU-mapped path.

## References

- `docs/esop-software-prd.md`: FR-016
- `docs/ethercat-master-requirements.md`: MBX-004
- `docs/esop-etg-cia402-master-requirements.md`: resilient mailbox polling
