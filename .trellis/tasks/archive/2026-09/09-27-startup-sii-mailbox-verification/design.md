# Design: Startup SII mailbox verification

## 1. Profile and comparison boundary

`StartupSlaveProfile` gains `expected_mailbox: Option<MailboxConfig>` plus a const
builder. Startup validates configured values during `start_inner` before replacing any
controller state.

The SII header cannot describe polling, retry, timeout or Status Bit policy. Add a
single mailbox-layout comparison helper on `MailboxConfig` that compares only send and
receive address/capacity. Both expected and observed configurations still pass the
shared full range validator before comparison.

`StaticProductConfig::startup_profiles` attaches each generated mailbox value and
returns a product-level typed error if the generated configuration is invalid.

## 2. Startup state machine

Add `StartupPhase::ReadingMailbox` and `StartupAction::SiiMailbox(SiiAction)`. Keep the
existing `StartupAction::Sii` variant for identity reads so pending-action equality and
request-pool matching reject responses from the wrong SII owner.

`StartupController` owns one `SiiBlockReader<SII_STANDARD_MAILBOX_WORD_COUNT>` and one
fixed `[Option<MailboxConfig>; MAX_SLAVES]` evidence array. After identity and topology
verification:

1. no expected mailbox: enter the existing AL path immediately;
2. expected mailbox: start the exact five-word block read for the current slave;
3. on completion: parse with `SiiStandardMailbox::from_completed_block`, require CoE,
   create the observed `MailboxConfig`, validate and compare its physical layout;
4. only on equality: publish the observed value and enter AL transition handling.

The reader uses the current Startup generation, station address,
`identity_timeout_ns` as the bounded SII discovery budget, and
`request_timeout_ns` for each control request. It is reset on each Startup start.

## 3. Errors and evidence

Add explicit Startup errors for invalid expected mailbox profile, SII mailbox
read/parse failure, and layout mismatch carrying slave position plus expected/observed
values. Lower-layer `SiiBlockError`, `SiiMailboxError`, and `MailboxConfigError` remain
intact through wrappers.

`verified_mailbox(position)` returns `Some` only after a complete valid match. A
mismatch never writes the evidence array. Startup's existing first terminal error
behavior remains unchanged.

## 4. Compatibility

- `StartupSlaveProfile::EMPTY` and `new(position)` have no mailbox expectation.
- `StartupController::start` keeps the exact current sequence.
- Existing explicit profile callers compile unless they use struct literals; current
  repository callers use constructors/builders.
- Generated product startup now opts into the additional read automatically.
- No Startup phase is renumbered on wire; these are Rust enum values without a stable
  external representation.

## 5. Verification and rollback

Unit tests drive the real EEPROM action sequence for the five words and assert that no
AL action appears before a successful match. Product tests assert generated profile
propagation. Existing Startup, scheduled service and lifecycle tests prove the opt-out
path remains unchanged.

Rollback is one implementation commit: no persisted runtime data, schema, generated
artifact bytes or ProcBuf ABI changes are introduced.
