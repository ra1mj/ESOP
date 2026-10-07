# ESOP V1 Readiness Audit

Date: 2026-10-07. The initial code and remote audit used commit
`c7d43e51ed0a5e12107bad8f55267087801568fe`. This report incorporates the
repository cleanup, executable CLI fixes and build-metadata corrections on
`codex/repository-cleanup-cli-audit`. Measurements below use Linux x86_64,
Rust 1.97.0, locked dependencies and the default release profile.

## Verdict

| Level | Verdict | Reason |
| --- | --- | --- |
| Library/simulator development and current read-only CLI | Usable within documented boundaries | Required local checks and actual command processes pass; a typed provider is still required |
| Software-complete V1 RC aligned with SOEM 2.0 | **NO-GO** | Missing protocol families, redundancy, ENI, maintenance tools and packaged runtime services |
| Hardware-qualified formal V1 | **NO-GO** | Software gaps plus unqualified target resources, interoperability, timing and soak/fault evidence |

The core has useful fixed-capacity and dependency boundaries, but the complete
host stack is not small or ready for simple installed operation. Cleanup and
a green CI run do not make missing functionality complete. This is a
capability/workflow audit, not a proof of every line or physical conformance.

## Confirmed defects repaired

| Defect | Result and direct evidence |
| --- | --- |
| Every real diagnostic process panicked at startup | CLI now uses the supported Tokio multi-thread scheduler with one worker; binary tests launch `CARGO_BIN_EXE_esop` |
| cfggen help returned usage failure | `--help`/`-h` return 0 on stdout; executable generation matches all six golden artifacts and invalid inputs still fail |
| Incident-only watch updates were hidden and doctor stayed healthy | New incidents are rendered without inventing a new State; doctor incorporates error incidents and returns 3 |
| Session initialization could outlive the advertised query timeout | Open/close use blocking workers because Zenoh 1.10.1's builder calls synchronous `wait` from `IntoFuture`; a nonresponding TCP handshake is tested against the CLI's own deadline |
| Closed output pipes could panic | Normal downstream pipe closure terminates successfully; other output failures return 8 |
| Rust 1.85 declaration contradicted code/dependencies | Declared and exercised minimum is 1.88; CI includes an explicit all-targets/all-features and bare-metal MSRV job |
| MIT license file contradicted Apache package metadata | Root MIT grant is retained; workspace package metadata now agrees. This is metadata consistency, not a new third-party/source-ownership qualification |
| Trellis repeatedly reopened approval and borrowed another session's task | Session isolation and conditional planning routing fixed; ten regression tests are part of `make ci` |

Raising the declared MSRV also enabled existing Clippy style diagnostics.
Mechanical let-chain and integer-divisibility suggestions were reviewed and
applied; bounded service ownership, priority and lifecycle policies remain
covered by the full regression suite.

## Verification

- `make ci`: 748 initial Rust tests plus 36 repeated product-config tests;
  98 Python tests; six router tests intentionally deferred to the live gate.
- `make test-zenoh`: all six live router tests pass, including four CLI suites.
- Rust **1.88.0**: `cargo check --locked --workspace --all-targets --all-features`
  and the `aarch64-unknown-none` core/product-config check pass.
- `make core-check`, `make core-example`, `make host-build`, `make footprint`
  and the existing cfggen golden-output gate pass.
- CLI coverage includes all six views and their finite watches; help/usage;
  healthy/degraded doctor; absent and slow providers; malformed Protobuf;
  wrong schema/robot/boot; malformed Domain/enum; stale State; provider boot
  changes; incident-only updates; empty deltas; stalled startup and closed pipes.
- No new privileged eBPF attachment, physical EtherCAT/HIL, target WCET,
  production credentials, safety or release-signing qualification is claimed.

Executable evidence lives in `crates/esop-cli/tests/command.rs`,
`crates/esop-cli/tests/zenoh_live.rs`, `crates/esop-cfggen/tests/generation.rs`,
and `scripts/tests/test_trellis_workflow.py`. Exit-code/timeout semantics are
specified in [the CLI contract](esop-runtime-diagnostics-cli.md).

## SOEM comparison


Pinned baseline: [SOEM v2.0.0](https://github.com/OpenEtherCATsociety/SOEM/tree/304d1c05eab77dc0d426f1a5cf09c8cc7dc03713).
Upstream tree and tag commit were rechecked. [the frozen inventory](history/soem-v2-baseline.md)
contains the original inventory. Coverage below refers to software behavior;
all target physical interoperability/performance remains **not-qualified**.
A `partial` row must not become `covered` merely because its existing subset
has unit tests. File references are relative to the repository root.

| SOEM family / upstream input | ESOP status | Current source and test evidence | Remaining closure |
| --- | --- | --- | --- |
| AP/FP/B/L/ARMW/FRMW raw commands; `include/soem/ec_base.h`, `src/ec_base.c` | partial | All 14 wire command opcodes at `crates/esop-ethercat-core/src/wire.rs`; frame/control/engine tests and `tests/cycle.rs` | Complete public operation/workflow matrix, compound LRWDC equivalence, operator access |
| NIC init/adapter discovery; `ec_main.h`, `oshw/` | partial | `LinuxRawPort::open`, `crates/esop-ethercat-linux-port/src/lib.rs`; simulator tests in `tests/master_sim.rs` | Adapter enumeration, runtime service and qualified physical paths |
| Scan/address/AL; `ec_config.h`, `ec_main.h` | partial | Core `scan.rs`, `al.rs`, `startup.rs`; startup/AL unit tests, Linux `tests/scheduled_domains.rs` | Usable service/slave CLI, broader device workflows and interoperability |
| SII categories/EEPROM read; `ec_main.h` | partial | Core `sii.rs`, `sii_discovery.rs`, `sii_stream.rs`; unit and startup/simulator tests | Generic tooling and full EEPROM write/ownership operations |
| EEPROM write/master-PDI ownership; `ec_main.h`, eepromtool | missing as complete capability | Existing typed SII readers above; core public module inventory `src/lib.rs`; no corresponding CLI | Bounded ownership/write/readback and maintenance workflow |
| SM/FMMU/PDO mapping, groups, overlap; `ec_config.h` | partial | Core `mapping.rs`, `pdo_config.rs`, `domain_registry.rs`; `tests/domain_registry.rs`, Linux scheduled-domain tests | Explicit SOEM group/overlap behavior equivalence and full workflows |
| Cyclic process data/WKC; `ec_main.h` | partial | Core `engine.rs`, `scheduled_domains.rs`, `schedule.rs`; `tests/cycle.rs`, Linux scheduled-domain/WKC tests | Full upstream mode matrix and hardware qualification; existing exact-WKC/generation contracts must remain |
| Generic mailbox and cyclic queues; `ec_main.h` | partial | Core `mailbox.rs`; Linux `tests/scheduled_domains.rs` | Per-slave multi-client broker, fairness, cancellation/admission and mixed-protocol load |
| CoE SDO and Complete Access; `ec_coe.h` | covered for implemented bounded software transfers | Core `coe.rs`; CA unit tests in the Complete Access tests and Linux `tests/slave_copy_sim.rs` | Full device compatibility and operator read/write workflow remain open |
| CoE OD/OE information; `ec_coe.h` | partial | Object-list/description/entry APIs at `sdo_information.rs`; fragment tests at `:1235`; product-selected PDO metadata verification | Complete public browsing/CLI workflow; these existing primitives are not absent |
| CoE mailbox RxPDO/TxPDO and mapping discovery; `ec_coe.h` | partial | Service IDs in `coe.rs`; generated PDO configuration and readback in `pdo_config.rs` | Dedicated mailbox services and generic discovery; enum IDs alone are insufficient |
| FoE; `ec_foe.h`, `src/ec_foe.c` | missing | Mailbox protocol tag only at `mailbox.rs`; no dedicated public controller/module | Independent bounded transfer, errors/progress, CLI and tests |
| SoE; `ec_soe.h`, `src/ec_soe.c` | missing | Mailbox protocol tag only; no dedicated public controller/module | IDN read/write/fragment/mapping behavior and tests |
| EoE; `ec_eoe.h`, `src/ec_eoe.c` | missing | Mailbox tag/test framing only; no Ethernet service | IP parameters, fragmentation/reassembly and optional bounded host TAP integration |
| DC topology/offset/delay/SYNC0/SYNC1; `ec_dc.h` | covered for implemented software paths | Core `dc.rs`; Linux scheduled-domain tests at `:4069` | Physical clock correctness and target timing remain not-qualified; retain P0/lock gates |
| Dual-NIC redundancy; `ec_main.h`, `oshw/*/nicdrv.c` | missing | Current single-interface LinuxRawPort at `src/lib.rs`; no dual-port runtime | Duplicate/reorder ownership, cable-failure paths and DC sample authority |
| Recovery/reconfiguration; `ec_config.h` | partial / intentional safety difference | Core `recovery.rs`, `state_request.rs`, `rescan.rs`, `reconfigure.rs`; scheduled-domain integration; the capability manifest states limits | Supervised complete workflow; reconfiguration remains PREOP and never automatically rearms motion |
| ENI; `scripts/eniconv.py`, `cmake/AddENI.cmake` | missing | cfggen currently consumes product JSON and ESI (`src/main.rs`, `esi.rs`, `generator.rs`) | ENI InitCmd conversion/validation and static outputs |
| Build capacities; `ec_options.h.in`, CMake options | partial | Rust const-generic capacities, cfggen model/generator; `tests/generation.rs`, product-config generated tests | Explicit mapping of every relevant upstream option to a generated product contract |
| Error records/tool output; `ec_print.h` | partial | Core `diag.rs`, bounded recovery diagnostics, typed host status | Full error/service/maintenance projections and usable CLI |
| Platforms/C ABI | different-by-design for proposed V1 scope | `no_std` core `src/lib.rs`, port abstraction, Linux development port; bare-metal check passes | First delivery proposed Linux x86_64 PREEMPT_RT; no C source/ABI or all-OS parity claim |

Full API/function-by-function closure remains a parity implementation gate;
this family matrix establishes the current NO-GO and the work inventory, not
an invented completeness percentage.

### Upstream sample workflows

| Pinned sample | ESOP equivalent now | Verdict |
| --- | --- | --- |
| `samples/ec_sample` | Core/Linux simulator tests; no packaged master daemon | partial |
| `samples/simple_ng` | Cyclic/DC and lifecycle libraries; no simple installed runtime startup | partial |
| `samples/slaveinfo` | Scan/SII/SDO Information primitives; no `slave info` command | partial |
| `samples/eepromtool` | Bounded EEPROM readers; no read/write/owner tool | missing complete workflow |
| `samples/firm_update` | No FoE transfer/tool | missing |
| `samples/eoe_test` | No EoE/IP/TAP workflow | missing |
| `samples/eni_test` | No ENI ingestion workflow | missing |


## Designed versus implemented CLI

The six current read-only views (`status`, `domain list`, `dc`, `lifecycle`,
`incident list`, `doctor`) and their `watch` forms are implemented and tested
through the actual executable. They query a running typed provider; a router
alone cannot generate runtime status. `esop-cfggen` generates product artifacts.

The wider V1 command design still has these missing namespaces:

| Namespace | Designed operations | Current executable status |
| --- | --- | --- |
| adapters | enumerate | absent |
| slave | list/info/state/recover/reconfigure | absent |
| eeprom | read/write/dump/owner | absent |
| sdo | read/write/list/describe | absent |
| pdo | map/send/recv | absent |
| foe | upload/download | absent |
| soe | read/write/map | absent |
| eoe | ip/bridge/status | absent |


Actual representative invocations reject those namespaces with usage exit 2.
No placeholder command is counted as implemented. `parse_view` in
`crates/esop-cli/src/lib.rs` is the current registry.

## Repository hygiene

The initial snapshot had 621 tracked files totaling 6,970,596 bytes. Removed
from the current Git tree: **286 archived task files and three personal journal
files**, totaling **1,034,715 bytes**. All 289 original local files were retained
and verified against their frozen SHA-256 and the baseline Git objects.

The 24 dedicated research/acceptance/run-result records were consolidated into
three retained documents, with original citations, run/commit/artifact details,
source hashes and qualification limits. The [development history index](development-history.md)
links all 66 complete task records at the immutable baseline and preserves the
completed diagnostic child's evidence pointer. Current contracts remain in
source-facing specs. Historical planned/missing statements are explicitly dated.

Ignore rules cover local completed-task/personal-workspace records, environment
secrets, keys, logs and editor leftovers; journal/archive auto-commit is disabled.
Source, tests, golden inputs, `Cargo.lock`, CI, shared workflow tooling, current
specs and active plans remain tracked. Build and CodeGraph outputs were already
local-only. Normal commits retain old Git objects; this cleanup does not shrink
history or assert that any old secret has been purged. The initial limited
current-tree secret-pattern scan found no matching keys/tokens.

## Simplicity and footprint

The workspace has 15 crates. Isolated core and product-config graphs each have
**8 external normal/build packages**; the CLI has **338**. The core graph does
not bring Zenoh, Tokio, Aya or host IPC into activated real-time code.

| Host artifact | File bytes | Stripped-copy bytes | ELF text | ELF data | ELF BSS |
| --- | ---: | ---: | ---: | ---: | ---: |
| `esop` | 24,382,728 | 17,165,976 | 16,576,445 | 582,616 | 5,044 |
| `esop-cfggen` | 1,531,032 | 1,186,280 | 1,157,458 | 25,408 | 2,952 |
| `wire_round_trip` | 454,600 | 349,256 | 332,883 | 13,008 | 4,266 |

The corrected CLI grows by 0.42% relative to the
24,279,864-byte baseline; bounded session work and actual error handling account
for that small increase. No transport or security feature was removed to make
this number smaller. A controlled startup wait for an unresponsive loopback
Zenoh handshake sampled **11,244 KiB RSS** and **5 threads** at 5 ms
intervals (393 samples); the process exited with timeout code 5 after
2.007 seconds. This is a sampled startup scenario, not a peak memory bound,
steady-state provider/watch benchmark or product memory budget.

`wire_round_trip` links only frame encoding/parsing and host `std`; its file
size is not the size of a complete master or an embedded image. Target flash,
RAM, stack/DMA/ProcBuf budgets and WCET still require a selected product/target.
Only `esop` and `esop-cfggen` are regular workspace executables; no masterd,
supervisord, systemd units or complete tar.zst distribution exists yet.

Reproduce with `make footprint`; the script writes artifact hashes, sections,
dependency identities, compiler/profile and sampling limits to
`build/footprint_report.json`. It strips temporary copies and preserves any
named gateway/raw-port markers discovered in them. This is ELF inspection,
not proof that a stripped future runtime package still permits probe attachment.

New `make core-check`, `make core-example` and `make host-build` entries separate
minimal development from host tooling. README now starts with working commands
and the provider boundary; the detailed [framework overview](framework-overview.md)
preserves implementation and qualification notes. The important next size
work is measuring the actual deployable runtime and selected host features,
not merging bounded controllers merely to reduce file counts.

## Ordered V1 closure

| Step | Dependencies / ownership | Automated evidence required | Hardware / release boundary |
| --- | --- | --- | --- |
| Runtime foundation | Single NIC owner, master service, supervisor, bounded local maintenance admission; parity workstream | Process startup/stop, IPC/state handoff, auth/admission failure and recovery tests | Product deployment/ACL review; missing services block RC |
| Raw/EEPROM/CoE and mailbox broker | Existing startup/AL/WKC owners; fair per-slave arbitration and generated capacities | Ownership/write/readback, OD/OE/PDO workflows, cancellation and mixed-client fairness under Domain/DC priority | Real-device interoperability; missing complete workflows block RC |
| FoE then SoE then EoE | Shared broker and maintenance admission; reuse existing RX/DC scheduler | Bounded transfer/reassembly, timeout/error/progress, full API and command process tests | Device/firmware/IP compatibility; each absent family blocks full SOEM RC |
| ENI conversion | Normalized product configuration; XML stays on host | InitCmd/static-output/validation/golden tests | Product ENI interoperability; absent ENI blocks full scope |
| Redundancy and supervised recovery | Dual-port RX duplicate authority and DC sample ownership | Cable-path, duplicate/reorder, fault/recovery simulation; never automatic motion rearm | Real cable/device faults and clock behavior |
| Software-complete RC | All software rows, maintenance CLI, daemon packaging and coexistence | Full feature build, executable command matrix, reproducible signed candidate and package installation | Not achievable from the current subset |
| Formal V1 | Frozen RC plus target/product/topology | Consistent evidence manifests, source/license/safety review | Real multi-vendor HIL, timing/resources, faults and long-duration soak; not measured here |

The cleanup/audit task closes confirmed hygiene/tooling defects and supplies
this inventory. Full protocol/service implementation remains with
`.trellis/tasks/09-28-soem-dc-parity`; the full-build contract must retain every
in-scope SOEM family even when smaller configured subsets are supported.
