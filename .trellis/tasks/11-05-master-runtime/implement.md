# Execution and verification

1. [x] Synchronize README archive/journal commits to main.
2. [x] Core PDO verification-only operations and typed product capabilities.
3. [x] ESI/config generation, optional CoE/static IO and bit layout; v2 artifacts.
4. [ ] OpOnly runtime state actions and recovery integration.
5. [ ] Reusable no_std runtime and independent public API tests.
6. [ ] Build-bound Linux entry and r0_r2 refactor with capture/evidence reuse.
7. [ ] Focused/full checks, specs/docs/resources, Q workloads and soak.
8. [ ] Commit/push, exact Linux CI, main integration and task closeout.

Use PATH=/Users/ra1mj/.cargo/bin:$PATH. macOS cannot run the Linux-only raw/Aya
workspace targets; run native focused tests/no_std plus actual Linux GitHub CI.
Do not stage paseo.json. Preserve old SDO default and ordinary controller APIs.


Increment 1: typed optional mailbox/CoE, fixed/assignment/configurable mapping,
per-PDO Fixed, runtime/product-config v2, bit layouts, physical mapping builder
from atomically verified SII, optional report capability records. Rebuilt all
seven product sets. r0_r2 now uses the public mapping builder. Native core /
product / cfggen tests, Clippy, report tests, no_std and all 13 software cases
plus two exact replays pass. A 1,000-cycle full smoke reached startup but was
too short for the scenario acceptance; the proper 10,000-cycle run passed.
Independent mixed/static bit-slave public-runtime scenarios remain part of
steps 5–7; hardware qualification remains pending.
