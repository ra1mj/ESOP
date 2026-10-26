# Progress checkpoint

User approved full implementation; scope Linux software/simulation, MCU and real
HIL deferred. Active task context: codex-esop-linux-r0-r2. Branch:
feat/linux-r0-r2-master. User-owned paseo.json was untracked on entry; do not stage.

## Implemented and independently verified

- Wire M/WORD layout, bootstrap BRD count, AP/BR transit-vs-WKC policies, scan
  timeout faults, shared RX/control/DC checks. Literal tests all14 opcodes.
- Reserved control wire-index leases separate logical FSM identities;1024 SII
  actions coexist with frozen Domain/DC without conflicts.
- Physical SMActivationC3 mask keeps SII OpOnly metadata out of reserved bits.
- Odd SII FMMU word padding normalized only with exact frozen odd expected count.
- Correct PDO mapping UNSIGNED32 descriptors and per-ESC/direction WKC; all
  generated baselines refreshed. Runtime SDO capacities frozen in artifacts.
- Full mailbox/MRL/CoE, bank8x1KiB, lazy preparation, optional-class round robin,
  mapped status after RX, linear configuration/repair tokens, terminal event ring.
- AL/DL monitoring with lifecycle projection, explicit maintenance FaultReset
  pulse+accepted-zero pipeline and no automatic lifecycle/motion rearm.
- Independent no_std simulator and full generated-product driver. Actual Q1
  32axes/32slaves1Domain, Q2/Q4 16axes/17slaves2Domains;904/459/459PDO bytes.
- CSP/CSV/CST dual-axis+IO, stop/rearm, six fault scripts, sustained8SDO load,
  virtual I2C NACK/timeout lifecycle isolation. Real allocator instrumentation.
- Classic PCAP+sidecar exact replay, full streaming wire hash despite prefix
  capture overflow; RTT observer uses port TX/poll times, not CPU receive time.
- Artifact-bound measurement importer (always unqualified software), strict
  Q1/Q2/Q4 release gate with verified Q4 baseline, build-resource validation.

## Final validation checkpoint

All implementation ownership is released. Final review also fixed legacy
mailbox wire fairness and sticky CoE readiness, post-RX mapped evidence for both
mailbox APIs, and full finite-feedback preflight before maintenance reset TX.
Core 379 unit tests, production SDO 12 tests, guard 65 tests and focused Clippy
passed after these fixes. Native macOS cannot run Linux-port gated tests;
portable legacy suites ran 55/55 and Linux-target checks/Clippy passed.
SSH orin initially reachable but lost route during isolated Rust install under
/tmp/esop-r0r2-validation; first linux-zerotier has changed hostkey, NOT bypassed.
GitHub gh auth available; use branch CI for realLinux makeci afterlocalgreen.

The short software gate passed all 13 cases, two exact replays, incomplete
capture rejection, modified TX rejection, and full counter preservation after
capture-prefix overflow. All 100 Python tests pass. Capability (45 entries),
Protobuf, performance report and development R2 gates pass. The explicit
require-qualified release gate correctly rejects the current unqualified
baseline. Release example builds. Original P0 evidence matrix is in
docs/esop-linux-software-master.md.

All 36 generated artifacts across six products match; all six strict C11
consumers compile and execute. Portable workspace check/Clippy pass; 771 tests
pass with two Linux-only packages excluded, three zenohd-required tests ignored
and one macOS socket-limit case filtered. Formatting and no_std checks pass.
Rust 1.99 deprecated AtomicU8::fetch_update; the gateway now uses an equivalent
MSRV-compatible CAS loop, with its existing 15 unit tests passing.

The release Q4 soak passed 3,600,000 cycles (one hour virtual time), 35,520 SDO
completions, zero cyclic/DC/mailbox errors and zero allocations/deallocations/
reallocations after activation. Eight requests remain explicitly outstanding at
shutdown. Capture contains the first 100,000 of 22,084,638 records; full-stream
digests and complete counters survive the explicit prefix loss. The importer
accepts it as unqualified software evidence. Compact evidence is versioned at
qualification/software_master_soak.json, bound to work commit 6efc82c.

Work commits 6efc82c (master), 235a5b5 (MSRV-compatible gateway CAS), and 585de61
(review, evidence and docs) are pushed to origin/feat/linux-r0-r2-master.
The initial OAuth workflow scope rejection was resolved by the user refreshing
authorization. Actual Linux run 40278257137 on head 585de61 passed all 12 jobs:
full make ci, BPF build, all ten privileged runtime qualifications, live Zenoh
router integration and evidence uploads. Linux workspace tests: 852 passed,
0 failed, 3 initially ignored router tests all passed in the dedicated step.
The previously filtered macOS IPC case passed unfiltered on Linux. Additional
product tests passed 38/38; Python validators 100/100. Thirteen software cases,
two exact replays and negative capture checks passed. Evidence artifact
12631082275 contains 153 files. All approved software acceptance criteria pass.
Run: https://github.com/ra1mj/ESOP/actions/runs/40278257137
Only task/journal bookkeeping remains after the verified code commits.
macOS Apple Clang cannot run the Linux BPF syntax gate; verify it on Linux CI.
Do not claim hardware qualification; current release gate must reject baseline.

Recent root monitor test fixture correction: cadence is peraction. Use15ns for
ALat0/DLat20/Idleat30 with maxage25, not100ns (which postpones DL beyondrun).
