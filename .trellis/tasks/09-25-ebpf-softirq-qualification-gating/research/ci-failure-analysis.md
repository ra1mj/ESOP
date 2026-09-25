# Bug Analysis: softirq qualification admitted ambient NET_RX work

## 1. Root Cause Category

- **Category**: E - Implicit Assumption
- **Specific cause**: the qualifier assumed that an exact CPU/vector predicate
  plus a prior quiet sample isolated the controlled injection. That predicate
  was active from runtime load through ring-buffer polling, so it provided
  spatial filtering but not temporal isolation. GitHub Actions run
  `36154809386` observed two records/incidents even though the injection's
  target-CPU `/proc/softirqs` delta had already passed the exact-one check.

## 2. Evidence and confidence

Initial hypotheses:

| Hypothesis | Prior |
|------------|------:|
| Ambient vector-3 work occurred outside the counter-bounded send interval | 65% |
| One GSO send created two target-CPU NET_RX actions | 20% |
| Runner/kernel/toolchain drift changed event semantics | 10% |
| The Trellis-only final commits changed runtime behavior | 5% |

Discriminating evidence:

- The failing log reported `records/incidents 2/2`, after BPF load and fixture
  compilation both succeeded.
- `GsoSockets::inject` would have failed earlier unless the selected CPU's
  counter delta around the GSO send was exactly one.
- The immediately preceding code commit passed the same dedicated job, while
  the final commits changed only task archive/journal files.
- Source inspection showed vector 3 remained active during capability setup,
  cycle publication, receive draining, and polling.

Updated confidence is above 95% that one admitted event occurred outside the
controlled counter interval. The fix therefore narrows the filter window rather
than adding retries or accepting extra records.

## 3. Prevention Mechanisms

| Priority | Mechanism | Specific action | Status |
|----------|-----------|-----------------|--------|
| P0 | Architecture | Load with an impossible exact vector, open vector 3 only around send/counter sampling, then close before receive/poll | Done |
| P0 | Test contract | Preserve exact one record/incident/statistics and add closed/open/closed report fields with mutation tests | Done |
| P1 | Documentation | Record that spatial selectors do not establish temporal isolation in backend quality guidance | Done |
| P1 | Remote integration | Require the dedicated privileged hosted job and independently validated artifact | Pending push |

## 4. Systematic Expansion

- **Similar issues**: a future hard-IRQ fixture cannot treat CPU/IRQ identity as
  temporal isolation; it needs a counter-bounded gate or a uniquely attributable
  producer. Other current qualifications use stronger identities such as exact
  TID, process TGID, unique veth ifindex/protocol, or explicit user markers.
- **Design improvement**: keep production observation broad, but make controlled
  qualification fixtures own a short admission window and publish its state.
- **Process improvement**: when a test claims exact event counts, compare the
  selector's lifetime with the stimulus lifetime, not only its value domain.

## 5. Knowledge Capture

- [x] Update `.trellis/spec/backend/quality-guidelines.md` with the temporal-gate
      contract and wrong/correct examples.
- [x] Update the report schema, validator, and mutation tests.
- [x] Update README, product PRD, observability design, and capability manifest.
- [ ] Capture hosted run and artifact measurements in `result.md` after push.
