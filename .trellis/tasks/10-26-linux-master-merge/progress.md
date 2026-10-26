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

Next: commit/push, verify final Linux CI and virtual-hour soak, fast-forward main
after fetching, verify main CI, archive/journal and push final bookkeeping.
