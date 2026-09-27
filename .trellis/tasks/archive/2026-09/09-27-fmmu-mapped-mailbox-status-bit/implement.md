# Implementation plan

## 1. Core contracts

- Add a fixed-size mailbox status FMMU binding with canonical constructor,
  validation, bit extraction, age policy, and exact `FmmuConfig` projection.
- Add explicit mapped polling mode and mapped observation transitions to
  `MailboxController` while preserving existing start/direct/PollTime APIs.
- Add focused core tests for active, inactive, unavailable, stale, and direct
  compatibility paths.

## 2. Domain observation

- Add a bounded `ScheduledDomainBank` lookup that validates Domain identity,
  committed quality, age, and bit bounds before returning an observation.
- Wire mapped observation into the standalone mailbox branch of the production
  service scheduler without affecting PDO configuration mailboxes.
- Extend production reports/progress only as needed to expose unavailable
  mapped status without allocating or blocking.

## 3. Generated product layout

- Pack status-enabled slaves by position after each Domain's PDO input region.
- Expand Domain input bytes, aggregate LRD length/WKC, cycle metrics, JSON,
  inventory, C, Rust, semantic identity, and committed golden artifacts.
- Emit one canonical mapped binding per supported slave.

## 4. Product validation and Mapping bridge

- Reconstruct the deterministic status tail and canonical FMMU descriptors in
  `esop-product-config`.
- Reject malformed declaration count, placement, overlap, Domain/datagram/WKC
  mismatch, age mismatch, or descriptor tampering before activation.
- Retain validated bindings in `ActivatedProduct` and expose lookup/accessors.
- Test the generated descriptor through the existing Mapping write/readback
  controller using Startup-style verified bank bounds.

## 5. End-to-end tests

- Update cfggen golden and product activation expectations.
- Add a generated-binding scheduled production test proving no direct status
  request, inactive suppression, active read, invalid-WKC suppression, stale
  suppression, and recovery on fresh input.
- Re-run direct Status Bit, PollTime, PDO batch, Domain, Mapping, and Startup
  regressions.

## 6. Documentation and quality gates

- Update product configuration, PRD status, EtherCAT requirements, capability,
  and software-plan wording without claiming HIL or conformance.
- Run focused crate tests during implementation.
- Run `cargo fmt --check`, generated artifact checks, `make no-std`, `make lint`,
  and full `make ci`.
- Complete Trellis acceptance, commit implementation, push to `ra1mj/ESOP`,
  archive the task, record the journal, and verify the exact final GitHub
  Actions run.
