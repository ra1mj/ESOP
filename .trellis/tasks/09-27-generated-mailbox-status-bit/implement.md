# Implementation plan

## 1. Canonical core policy

- Add shared SyncManager status-register constants or helpers in the core.
- Add a shared `const` `MailboxStatusBit` derivation for the canonical
  mailbox-full bit, validate the generated receive-SyncManager descriptor at
  the product boundary, and add focused unit tests.

## 2. Preserve ESI discovery evidence

- Extend `EsiMailbox` with send/receive SyncManager indices.
- Update parser validation, fixtures, and unit tests.
- Define one cfggen helper that determines whether ordered FMMU usages request
  the direct Status Bit policy.

## 3. Generate complete artifacts

- Extend JSON/inventory/C schemas and generated Rust initializers.
- Regenerate committed examples/golden files.
- Confirm semantic/config hashes change deterministically with the policy.

## 4. Product and Startup validation

- Extend generated `ProductSlaveConfig` data with mailbox SyncManager evidence.
- Re-derive and validate the canonical policy in product configuration.
- Pass the expected receive SyncManager descriptor into Startup.
- Cross-check the online SII category entry before publishing verified mailbox
  configuration.

## 5. End-to-end runtime coverage

- Add a generated-product production-scheduler test for inactive/active Status
  Bit behavior.
- Preserve and rerun PollTime and manual-policy tests.

## 6. Documentation and verification

- Update PRD/requirements/product-configuration/README status wording.
- Run focused tests while implementing.
- Run formatting, no_std checks, clippy, and full `make ci`.
- Complete Trellis acceptance, archive the task, update the journal, commit and
  push all work, then verify the exact final GitHub Actions run.
