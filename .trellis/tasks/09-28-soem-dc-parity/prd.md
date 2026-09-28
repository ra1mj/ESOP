# SOEM parity with DC preservation

## Goal

Close practical SOEM 2.x feature gaps in independently qualified increments
without regressing ESOP's existing DC, exact WKC, bounded scheduling, product
configuration, or fail-closed lifecycle behavior.

## Requirements

### R1. Capability order

Implement the remaining capabilities in this default order:

1. cyclic mailbox arbitration and multi-client request fairness;
2. FoE maintenance-mode file transfer;
3. SoE IDN read/write and mapping discovery;
4. EoE fragmented frame and IP-parameter transport outside P0;
5. cable redundancy with duplicate/reordered frame handling;
6. product-qualified supervised recovery and real-device interoperability.

Order may change only when HIL evidence or a concrete product requirement
justifies it. Each capability is its own child task and commit.

### R2. DC invariant

Every capability must demonstrate that cyclic FRMW, optional sync-window BRD,
Domain frames and control requests keep distinct indices, non-overlapping
process-image ranges, exact WKC, same-generation ownership, and unchanged P0
priority. Cable redundancy must preserve one authoritative DC reference sample
and must not merge duplicate clock responses.

### R3. Shared bounded transport

New protocols shall use typed fixed-capacity controllers, the existing mailbox
or control pool, absolute deadlines, and the production service scheduler.
They shall not allocate, block, sleep, or log in the activated cycle.

### R4. Product admission and feature gates

Optional protocols and redundancy require explicit product intent, generated
configuration evidence, static capacity, build-report projection, and a
compile-time feature where dependencies or code size are material.

### R5. Recovery safety boundary

SOEM-style automatic reconfiguration must not be copied as an unconditional
return to OP. ESOP recovery shall remain fail-closed: detect and diagnose
automatically, but require product policy and fresh lifecycle qualification
before any automatic state/configuration action or motion rearm.

### R6. Evidence

Each capability needs deterministic unit/integration tests, scheduler and DC
coexistence evidence, resource/performance impact reporting, and an explicit
statement of the remaining real-device/HIL boundary.

## Acceptance Criteria

- [ ] The ordered capability matrix maps SOEM behavior to ESOP modules,
  scheduler priority, product fields, feature gates and tests.
- [ ] Each implementation child includes a DC coexistence regression and P0
  plan-isolation assertion.
- [ ] Optional protocol failures remain request-local unless explicit product
  policy escalates them into lifecycle qualification.
- [ ] Recovery never restores motion authority from communication success
  alone.
- [ ] Capability manifest claims change only with matching source and test
  evidence.

## Out of Scope

- Implementing all parity capabilities in one change.
- Weakening WKC, generation, deadline, lifecycle, or product-validation rules
  to mimic permissive example behavior.
- Claiming ETG conformance, target WCET, real-device interoperability, cable
  redundancy, or functional safety before their dedicated evidence passes.
