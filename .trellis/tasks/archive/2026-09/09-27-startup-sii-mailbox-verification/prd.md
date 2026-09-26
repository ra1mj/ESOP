# Integrate live SII mailbox verification into Startup

## Goal

Close the next software activation gap after generated mailbox discovery. When a
startup profile carries a generated CoE mailbox configuration, `StartupController`
must asynchronously read the standard SII mailbox header from the addressed slave,
validate it through the existing strict parser, and require its physical mailbox
layout to match the generated product data before beginning AL transitions.

This task proves bounded online EEPROM acquisition and software comparison. It does
not prove that a physical device response is authentic, interoperable, timely on a
target, or qualified for release.

## Background

- Generated `ProductSlaveConfig` now owns one validated CoE `MailboxConfig` per slave.
- `StaticProductConfig::startup_profiles` already carries generated ESM timeout and
  `OpOnly` data into `StartupController`, but it does not carry mailbox expectations.
- `SiiStandardMailbox` already parses exactly words `0x001C..0x0020`, checks CoE and
  converts slave receive/send fields into master send/receive direction.
- `SiiBlockReader` already provides allocation-free asynchronous EEPROM reads through
  the same fixed control-request path used by Startup identity discovery.
- Startup currently transitions directly from verified identity to AL state changes,
  so a configured product can reach PREOP/SAFEOP/OP without comparing generated ESI
  mailbox ranges with the online SII values.

## Requirements

### Startup profile contract

- Extend `StartupSlaveProfile` with an optional expected mailbox configuration.
- Validate every configured mailbox before mutating Startup state or emitting a
  control action.
- Compare only the SII-representable physical layout: master send/receive address and
  capacity. Poll interval, timeout, retry and optional Status Bit policy remain runtime
  policy and must not cause an SII layout mismatch.
- Preserve the existing `start()` and profiles without an expected mailbox as a
  source-compatible path with the current identity-to-AL sequence.

### Online SII verification

- Add an explicit Startup phase and action owner for the standard five-word mailbox
  range so identity and mailbox reads cannot be confused or cross-consumed.
- Read the exact range from each profiled slave after identity verification and before
  any AL transition for that slave.
- Reuse `SiiBlockReader`, `SiiStandardMailbox`, `SiiMailboxError`, and shared
  `MailboxConfig` validation; do not add a second EEPROM or range parser.
- Use the existing bounded SII timeout/request-timeout budget and preserve generation,
  token, datagram shape, WKC, deadline and control-request ownership checks.
- Require CoE support and exact physical-layout equality. Any read, parse, validation
  or mismatch error must latch Startup `Faulted` and prevent AL actions and Ready.
- Publish verified mailbox evidence only after the complete range parses and matches;
  expose it by slave position without allocation.

### Product propagation

- `StaticProductConfig::startup_profiles` must validate and attach each generated
  mailbox configuration to the matching startup profile.
- Invalid generated mailbox data must fail before Startup mutation.
- The checked-in generated drive/drive/IO configuration must produce profiles with the
  exact generated mailbox values.

### Evidence and documentation

- Add focused core tests for exact range acquisition, direction mapping, successful
  verification, mismatch, missing CoE, timeout/error propagation, action separation,
  multiple-slave ordering and legacy opt-out behavior.
- Add product-config tests for generated profile propagation and invalid configuration
  rejection.
- Update product configuration, EtherCAT requirements, software PRD, capability
  manifest and Trellis contract without claiming physical authenticity or HIL.
- Preserve `no_std`, fixed arrays, caller-owned control requests and zero allocation.

## Acceptance Criteria

- [ ] A profile with an expected mailbox causes Startup to read exactly SII words
      `0x001C..0x0020` after identity and before the first AL action.
- [ ] Valid CoE SII data with the exact generated address/capacity layout is retained as
      verified per-slave evidence and Startup proceeds normally.
- [ ] Address, capacity, direction or CoE mismatch returns a typed error, latches
      `Faulted`, emits no AL action and publishes no verified mailbox.
- [ ] SII token/generation/WKC/length/deadline/control errors remain fail-closed through
      the existing request-pool contract.
- [ ] Runtime-only mailbox policy differences do not cause a physical-layout mismatch.
- [ ] Profiles without mailbox expectations and the legacy `start()` API preserve the
      existing identity-to-AL behavior.
- [ ] Generated product startup profiles carry the exact drive/drive/IO mailbox data
      and reject invalid mailbox configurations before Startup mutation.
- [ ] Focused tests, generated artifact checks, workspace formatting/lint/tests,
      `aarch64-unknown-none`, BPF, simulated HIL and Zenoh gates pass.
- [ ] Documentation states that physical response authenticity, real-device
      interoperability, target WCET and hardware HIL remain open.

## Out Of Scope

- Reading complete SII SyncManager/FMMU/PDO/DC categories during Startup.
- Replacing generated ESI data with live SII values or auto-repairing a mismatch.
- Mailbox Status Bit discovery, bootstrap mailbox, non-CoE protocol activation or
  Complete Access.
- Cryptographic device identity, signed EEPROM contents, ETG conformance, physical HIL,
  release qualification or functional-safety claims.
