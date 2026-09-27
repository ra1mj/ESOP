# ESC watchdog configuration and readback

## Goal

Close the remaining software-only ESC watchdog gap in FR-004 and CFG-005 by
making per-slave watchdog register intent part of the deterministic product
configuration, programming every configured value through the bounded control
request path, reading each register back exactly, and keeping the PREOP
configuration barrier closed until the complete product-order plan succeeds.

## Background

- The product generator and runtime already own deterministic per-slave PDO,
  SyncManager, FMMU, mailbox, DC, and lifecycle configuration evidence, but no
  product field or runtime service currently owns ESC watchdog intent.
- EtherLab IgH 1.6.13 commit
  `61cc654f5b721ddd54df0f58bdd34106d91c5359` documents and implements the
  standard raw register contract: Watchdog Divider at `0x0400/2` and Process
  Data Watchdog Time at `0x0420/2`; a zero API value means the register is not
  written and the ESC default remains in effect.
- Existing ESOP control services require exact action/generation ownership,
  bounded deadlines, WKC validation, typed first-failure retention, and
  complete-only evidence publication. Watchdog programming must follow the
  same contract and must not introduce allocation or blocking waits.
- Physical response provenance, actual watchdog expiration behavior, target
  timing, interoperability, and HIL evidence remain outside software-only
  acceptance.

## Requirements

### R1. Strict product-owned watchdog intent

`esop.product.v1` shall accept an optional strict per-slave `watchdog` object
with independently optional `divider` and `process_data_intervals` raw `u16`
values. At least one field must be present and every present value must be
nonzero. An absent object or absent field means preserve that ESC register's
default value and emit no write for it.

The normalized product, inventory, generated C/Rust contracts, semantic
configuration hash, and `no_std` `ProductSlaveConfig` shall carry the exact
intent. Unknown fields, explicit zero, or an empty object shall fail before
artifact publication. Existing manifests without `watchdog` remain valid and
retain their previous runtime behavior.

### R2. Fixed-capacity plan and controller

The core shall expose fixed-size `EscWatchdogConfig`, product-order plan entry,
plan, controller configuration, action, progress, programmed evidence, phase,
and typed error contracts. Plan construction shall reject invalid values,
duplicate positions, duplicate station addresses, and capacity overflow before
controller mutation.

The controller shall support an empty plan as an immediate successful no-op.
For each configured field it shall emit a fixed-address two-byte little-endian
write followed by a fixed-address two-byte read and advance only when the
readback matches exactly. Actions shall require WKC 1, exact response length,
one absolute configuration deadline, bounded per-request deadlines, and the
existing control pool ownership checks. Any action, generation, payload,
readback, WKC, timeout, deadline-overflow, or pool failure shall latch the first
typed failure and publish no programmed evidence.

### R3. Product plan attachment

`StaticProductConfig` shall validate every static watchdog value and build one
fixed-capacity plan in generated slave order using each slave's configured
position and station address. Invalid generated/static values shall fail before
Startup or controller mutation. A fully unconfigured product shall produce an
empty plan.

The checked-in simulator product shall exercise both watchdog registers on its
two drives while leaving the IO slave unconfigured, proving optional behavior
and deterministic generated artifacts.

### R4. Production scheduling and lifecycle gate

The production service scheduler shall own an optional watchdog controller as
a first-class service ordered after PDO Configuration and before Mapping. It
shall preserve one in-flight request across cycles, route completed/failed
control requests to the exact pending action, expose typed progress/fault and
readiness, and never allow another service to overtake the active transaction.

`StartupConfigurationServices` shall gain an explicit watchdog requirement.
When selected, a missing controller or any non-Complete/faulted controller
shall keep the PREOP barrier closed. Products and callers that do not request
the watchdog service shall preserve the existing service order and startup
traffic.

### R5. Documentation and evidence boundaries

README/product configuration/master requirements, capability manifest, and
backend executable product specification shall describe the exact schema,
register order, readback and lifecycle behavior. Claims shall remain limited to
deterministic software responses supplied by the caller; no physical watchdog
trip, response authenticity, interoperability, target WCET, HIL, ETG
conformance, or functional-safety qualification may be inferred.

## Acceptance Criteria

- [x] Core tests prove empty plans, optional individual fields, exact
      `0x0400/2` then `0x0420/2` write/read order, little-endian values,
      product-order traversal, complete-only evidence, and control-pool
      ownership.
- [x] Negative core tests cover zero values, empty config, duplicate
      position/station, capacity, action/generation substitution, short
      readback, WKC mismatch, value mismatch, timeout, deadline overflow, and
      explicit restart after fault.
- [x] Cfggen rejects unknown/empty/zero watchdog declarations and emits stable
      normalized JSON, inventory, C, Rust, and hash evidence for valid optional
      declarations.
- [x] Product runtime tests prove generated-order plan construction, exact
      position/station/value propagation, invalid static data rejection, and an
      empty-plan path for unconfigured products.
- [x] Scheduler and Linux simulation tests prove watchdog priority between PDO
      and Mapping, cross-cycle request retention/rebuild, exact readback
      routing, typed faulting, and PREOP release only after Complete.
- [x] Legacy products and Startup service masks without watchdog requirements
      retain their existing traffic and completion behavior.
- [x] `cargo test -p esop-ethercat-core --no-fail-fast` passes.
- [x] `cargo test -p esop-cfggen --no-fail-fast` passes.
- [x] `cargo test -p esop-product-config --no-fail-fast` passes.
- [x] `cargo test -p esop-ethercat-linux-port --test scheduled_domains` passes.
- [x] `make ci` passes.
- [x] The final commit is pushed to `ra1mj/ESOP`, and the exact pushed SHA has
      a successful GitHub Actions `quality` run.

## Out of Scope

- Calculating or promising a real-time watchdog duration from ESC-specific
  clock behavior; the product stores the raw standard register values.
- Enabling/disabling SyncManager watchdog mode bits or deriving watchdog intent
  from live SyncManager register discovery.
- Runtime retuning after activation, automatic retry policy beyond the existing
  control transport, or rollback of already accepted ESC writes.
- Physical watchdog expiration/trip testing, response provenance,
  interoperability, target resource/WCET evidence, HIL, ETG conformance, and
  functional-safety qualification.
