# Integration checkpoint

User explicitly requested continued improvement and merge to main. Existing
feature history is preserved; original user-owned paseo.json remains untouched.
Origin main was c7d43e5 and unprotected when inspected; fetch again before merge.

## Confirmed hardening

- Idle receive expiration releases mailbox poll ownership and uses one bounded
  additional work pass so both already-queued and later same-station SDOs run.
  Real scheduler/VirtualBus regressions verify deadline and Emergency retention.
- AL/DL quality carries sampled-at and inclusive earliest fresh-until times.
  Actual service owners re-age it around auxiliary/motion TX and publication,
  retaining accepted Controlword/stop truth. Five public pipeline cases pass.
- Outstanding TX index reuse permanently suppresses ambiguous RTT for the
  observer session. Missing observations are absent rather than zero filled.
  Actual per-cycle RTT coverage is exported; legacy v1 coverage stays unknown.
- Offline baseline must be completed, allocation-free and have zero submitted
  SDO work. A real failed imported run cannot define a regression reference.
- Canonical supported scenario validation precedes port setup; unknown Q3 no
  longer runs a different product. Lowercase aliases bind consistent artifacts.
  Added help. A 100-cycle 17-station alias test was too short to qualify its
  monitor/guard warmup; corrected to the established 1000-cycle workload.

## Checks completed

- Final cross-layer check: 545 core/sim/guard Rust tests in18 suites, Clippy and
  library checks pass; no remaining confirmed defect. Example16 tests and
  virtualI2C8 tests pass. Native/Linux-target example Clippy passes.
- Full local software gate:13 cases, canonical alias, invalid-before-port
  rejection, two exact replays, prefix/modified TX negatives all pass.
- Python tests103/103, no_std core/product/sim, capability45 and Protobuf pass.
- All55 legacy portable Linux fixtures pass; actual Linux gate remains needed.
- Prior head18933fa CI is fully green. New code requires its own final CI.

## Final code and evidence verified

Hardening commits acae720, 2beca02 and deaf254 are pushed. Actual Linux CI on
deaf254 passed all 12 jobs, including make ci, Zenoh integration, BPF and ten
privileged runtime checks:
https://github.com/ra1mj/ESOP/actions/runs/40283639418

The final-code Q4 soak passed 3,600,000 cycles, with 3,600,000 observed RTT samples,
35,520 completed SDOs, eight explicitly outstanding requests and zero activated
heap operations. All cyclic/DC/mailbox error counts are zero. The capture keeps
100,000 of 22,084,638 records; 21,984,638 omitted records remain explicit while
full-stream/state/event digests and cycle/error counts cover the whole run.
Wire/state/event digests match the previous soak. On 2026-11-02 the offline report
was regenerated against deaf254 and the versioned evidence/hash inventory updated.
Hardware qualification remains false.

Origin main was refreshed on 2026-11-02 and remains c7d43e5, unprotected and an
ancestor of the feature branch. Next: commit evidence, fast-forward/push main,
verify main CI, then archive/journal and push final bookkeeping.
