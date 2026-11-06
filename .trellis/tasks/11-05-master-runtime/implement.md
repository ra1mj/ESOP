# Execution and verification

1. [x] Synchronize README archive/journal commits to main.
2. [x] Core PDO verification-only operations and typed product capabilities.
3. [x] ESI/config generation, optional CoE/static IO and bit layout; v2 artifacts.
4. [x] OpOnly runtime state actions and recovery integration.
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

Increment 1 commit 7fbe8d2 is pushed; exact Linux CI run 42048297176 passed
all 12 jobs. Increment 2 adds composite StateRequestAction, retained OpOnly
binding, disable/verify before AL and enable/verify after OP with the original
step deadline, including same-state verification. AL observations keep their
actual sample time. The same production slot handles both action types;
legacy AL-only methods remain adapters. Core tests, Clippy and no_std pass.
Runtime public ownership/recovery APIs remain step 5.

Runtime implementation note: existing cycle owner and Domain bank deliberately
borrow the exact schedule by address. Use explicit caller-owned storage and
phased Startup -> Ready -> Runtime binding to avoid self-references or fabricated
observed slave records. MasterStartup can borrow prebuilt schedule/domain storage;
finish consumes it into Ready (real verified topology + ActivatedProduct + unique
SDO transport tokens). Ready.bind borrows the immutable active plan and mutable
controllers, with the external domain storage, into MasterRuntime. Keep the
production scheduler and ScheduledProductionCycleOwner. Pure IO also needs an
explicit disabled DC path; never treat WKC 0 as a measured clock.
