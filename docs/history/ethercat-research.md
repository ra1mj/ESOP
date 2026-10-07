# Historical EtherCAT Design Research

These are dated September 2026 records preserved during repository cleanup.
They retain their original findings, citations and qualification limits.
Statements about missing features or future work describe that historical
snapshot, not the current implementation. Current source, backend specs and
the qualification manifests take precedence. No new HIL or release claim
is established by copying these records. Original bytes remain available
at the pinned source links; only heading levels and a local compiler path
are normalized below.

<a id="09-26-al-error-acknowledgement-research-esc-al-error-acknowledgement-md"></a>

## 09-26-al-error-acknowledgement/research/esc-al-error-acknowledgement.md

[Original record](https://github.com/ra1mj/ESOP/tree/c7d43e51ed0a5e12107bad8f55267087801568fe/.trellis/tasks/archive/2026-09/09-26-al-error-acknowledgement/research/esc-al-error-acknowledgement.md) · SHA-256 `645705f003f01c0368ef61d697d7897c26f708f03b97bbeeb794f03b8c21ccc7`

### ESC AL Error Acknowledgement Research

#### Primary Source

Beckhoff Automation, *EtherCAT Slave Controller Hardware Data Sheet, Section I
- Technology*, version 2.3, 2017-02-21, chapter 10.1, page I-76.

Source:
https://download.beckhoff.com/download/document/io/ethercat-development-products/ethercat_esc_datasheet_sec1_technology_2i3.pdf

#### Relevant Protocol Facts

- AL Control is at `0x0120:0x0121`; AL Status is at `0x0130:0x0131`; AL Status
  Code is at `0x0134:0x0135`.
- Device Emulation is indicated by ESC Configuration `0x0141[0]`.
- With Device Emulation enabled, the ESC copies AL Control directly into AL
  Status. The master must not set Error Indication Acknowledge for that device,
  because it would synthesize the Error Indication bit even when no error
  occurred.
- A normal slave reports Error Indication at `0x0130[4]` and a code at
  `0x0134:0x0135`. The master acknowledges it through `0x0120[4]`.

#### Project Consequences

- Capability must be read explicitly before AL error handling policy is chosen.
- Device Emulation cannot share an unconditional acknowledgement path.
- The acknowledge command must retain the observed AL state in bits 3:0 and set
  only bit 4.
- Clearing Error Indication is not evidence that the failed transition is safe
  to retry. ESOP will retain a terminal startup fault and require explicit
  restart.

<a id="09-27-dc-offset-delay-programming-research-ethercat-dc-offset-delay-md"></a>

## 09-27-dc-offset-delay-programming/research/ethercat-dc-offset-delay.md

[Original record](https://github.com/ra1mj/ESOP/tree/c7d43e51ed0a5e12107bad8f55267087801568fe/.trellis/tasks/archive/2026-09/09-27-dc-offset-delay-programming/research/ethercat-dc-offset-delay.md) · SHA-256 `69000e4e06cd4f0a4821f89971adfd868fb4d462673b2d6cc80e779494c364e7`

### EtherCAT DC offset/delay research

#### Primary references

- Beckhoff ET1100 EtherCAT Slave Controller datasheet, register descriptions
  for System Time Offset `0x0920:0x0927` and System Time Delay
  `0x0928:0x092B`.
- Beckhoff Distributed Clocks documentation, propagation-delay and local-time
  offset behavior.
- EtherLab/IgH EtherCAT master FSM source, used only as an external behavior
  reference for reading System Time plus old offset and calculating 32/64-bit
  corrections. The ESOP implementation remains independently structured.

#### Derived implementation constraints

- Offset is a signed correction represented in a 64-bit ESC register; delay is
  an unsigned 32-bit nanosecond value.
- Offset and delay should be written together in one access so a slave does not
  observe a mixed pair.
- Reading 24 bytes from `0x0910` yields the current System Time and prior offset
  needed for incremental correction.
- A 64-bit clock uses the full signed application-minus-system delta.
- A 32-bit clock uses the signed low-32-bit wrapping delta.
- Elapsed monotonic time between the caller's application-time sample and the
  accepted ESC sample must be added before calculating the correction.

#### Claim boundary

These register operations initialize the software-visible DC relationship;
they do not prove the external application-time source, cable/device timing
precision, ongoing clock lock, or hardware conformance.

<a id="09-27-esm-timeouts-op-only-research-esm-timeout-op-only-md"></a>

## 09-27-esm-timeouts-op-only/research/esm-timeout-op-only.md

[Original record](https://github.com/ra1mj/ESOP/tree/c7d43e51ed0a5e12107bad8f55267087801568fe/.trellis/tasks/archive/2026-09/09-27-esm-timeouts-op-only/research/esm-timeout-op-only.md) · SHA-256 `d6c38fe4ef670c6166e02cd425c5742341c77758d832acaffae4cadcaf3b5408`

### Research: ESM timeout and OpOnly requirements

Research performed 2026-09-27. Sources are used to define implementation behavior, not
to claim conformance or certification.

#### Primary sources

##### ETG.1500 EtherCAT Master Classes, V1.0.2

Source:
`https://www.ethercat.org/download/documents/ETG1500_V1i0i2_D_R_MasterClasses.pdf`

- The master requirements state that ESM transition timeouts shall come from ESI/SII
  and use ETG.1020 defaults when absent.
- They require all `OpOnly` output SyncManagers to be disabled while a slave is not in
  Operational state.
- Both ESI and SII provide an `OpOnly` indication.

##### ETG.2000 Slave Information Interface, V1.0.15

Source:
`https://www.ethercat.org/download/documents/ETG2000_V1i0i15_S_R_SlaveInformationInterface.pdf`

- The SII SyncManager category encodes the initial enable flag and the `OpOnly` flag in
  the activation byte.
- Activation bit 0 is enable; bit 3 is `OpOnly`. They must not be collapsed into a
  single boolean.

##### EtherCATInfo schema evidence

Source:
`https://github.com/DiamondLightSource/ethercat/blob/master/etc/xml/EtherCATInfo.xsd`

This is a public repository copy of the EtherCATInfo schema, used as schema evidence
because current ETG.1020 content is member-restricted. It defines:

- `PreopTimeout` default: 3000 ms;
- `SafeopOpTimeout` default: 10000 ms;
- `BackToInitTimeout` default: 5000 ms;
- `BackToSafeopTimeout` default: 200 ms;
- SyncManager `OpOnly` as a boolean attribute mapped to the SII flag.

The implementation records these values in a named versioned constant. A future update
requires an explicit code/documentation change.

##### Beckhoff EtherCAT ESC technology data sheet

Source:
`https://download.beckhoff.com/download/document/io/ethercat-development-products/ethercat_esc_datasheet_sec1_technology_2i3.pdf`

- SyncManager register blocks use an 8-byte stride.
- The activation byte is at offset 6 within the block.
- Activation bit 0 controls SyncManager enable.

#### Decisions

- Use per-transition profiles with a non-zero uniform compatibility override.
- Share one absolute deadline across OpOnly and AL work for a transition step.
- Disable before leaving OP, but enable only after OP has been observed; startup readiness
  remains held until readback succeeds.
- Treat `OpOnly` on an input/TxPDO SyncManager as invalid configuration.
- Preserve raw SII activation flags and derive enable/OpOnly via bit helpers.
- Keep physical HIL and device interoperability as open evidence after software tests.

#### Deferred questions

- Device-specific init-command sequences may impose additional ordering constraints;
  those require physical HIL evidence.
- Complete SM/FMMU/DC descriptor discovery and mailbox configuration are separate PRD
  increments and are not bundled into this task.

<a id="09-27-ethercat-async-register-requests-research-igh-register-request-md"></a>

## 09-27-ethercat-async-register-requests/research/igh-register-request.md

[Original record](https://github.com/ra1mj/ESOP/tree/c7d43e51ed0a5e12107bad8f55267087801568fe/.trellis/tasks/archive/2026-09/09-27-ethercat-async-register-requests/research/igh-register-request.md) · SHA-256 `ee8e2d7e437da52ed0f16dd114c66cab514489338d62a04d5a281900e649dfc1`

### IgH Register Request Reference

#### Primary Source

IgH EtherCAT Master 1.6 `reg_request.c` implements reusable request objects
with a caller-visible state machine (`UNUSED`, `BUSY`, `SUCCESS`, `ERROR`), a
fixed data buffer allocated at creation, and explicit asynchronous read/write
queueing. Reads record station address, register offset and transfer size;
writes reuse the configured buffer and configured size.

Source inspected on 2026-09-27:
`https://docs.etherlab.org/ethercat/1.6/doxygen/reg__request_8c_source.html`

#### ESOP Adaptation

- Preserve the observable asynchronous state shape, but use caller-owned
  fixed-capacity arrays instead of allocation.
- Add stable generation handles because a shared controller owns multiple
  reusable slots.
- Keep explicit per-request deadlines and exact WKC/action validation required
  by the existing ESOP control plane.
- Execute through the existing production service scheduler so register access
  cannot create an unbudgeted side channel around PDO/DC traffic.

<a id="09-27-ethercat-explicit-requesting-id-research-protocol-md"></a>

## 09-27-ethercat-explicit-requesting-id/research/protocol.md

[Original record](https://github.com/ra1mj/ESOP/tree/c7d43e51ed0a5e12107bad8f55267087801568fe/.trellis/tasks/archive/2026-09/09-27-ethercat-explicit-requesting-id/research/protocol.md) · SHA-256 `4668c493cbf159ce6239080f2a3c5cf73b83a06ad10566a25fe0cc09e972eee3`

### Protocol Research

#### Requesting ID Sequence

Primary vendor documentation describes Requesting ID as an INIT-state sequence:

1. Set AL Control register `0x0120` ID Request bit 5.
2. Wait for AL Status register `0x0130` ID Loaded bit 5.
3. Read the identification value from AL Status Code register `0x0134`.

The write must retain the current AL state bits. The 16-bit value at `0x0134`
is product-configured and used to distinguish same-model devices.

Sources consulted 2026-09-27:

- Beckhoff EtherCAT system documentation, “Explicit Device Identification”.
- Beckhoff AX8000 diagnostics, “Identifying slave devices”.
- Microchip EtherCAT conformance test record for explicit requesting ID.
- HPMicro SSC ESI examples declaring
  `Device/Info/IdentificationReg134=true`.
- EtherCATInfo XSD copies showing optional `IdentificationReg134` boolean and
  separate optional `IdentificationAdo` value.

#### Scope Decision

Only Requesting ID through `0x0134` is implemented in this task. Configured
Station Alias and arbitrary Data Word identification are separate mechanisms
and remain explicit limitations. This avoids collapsing three different wire
contracts into one ambiguous configuration field.

<a id="09-27-ethercat-explicit-rescan-research-existing-startup-reuse-md"></a>

## 09-27-ethercat-explicit-rescan/research/existing-startup-reuse.md

[Original record](https://github.com/ra1mj/ESOP/tree/c7d43e51ed0a5e12107bad8f55267087801568fe/.trellis/tasks/archive/2026-09/09-27-ethercat-explicit-rescan/research/existing-startup-reuse.md) · SHA-256 `88b169a680bf21599300da2a235a1da121d88e16258ace0e8d95e1048c4093be`

### Existing Startup Reuse Evidence

#### Confirmed Authorities

- `ScanController` is restartable from Idle, Complete, or Faulted and clears
  its fixed record array before probing.
- `StartupController::start_inner` already performs the complete synchronous
  evidence reset required by rescan: scan/SII/register/AL controllers, slave
  table, verified arrays, reference clock, DC topology, OpOnly gates,
  configuration state, and faults.
- Startup retains the expected-slave and profile arrays after Ready/Faulted, so
  rescan can copy and reuse the compiled product plan without caller-provided
  dynamic data.
- Startup already accepts restarts from Ready and Faulted, but its public start
  APIs require callers to resupply the plan and can target OP. A named rescan
  wrapper must freeze PREOP and an empty configuration-service barrier.
- The production scheduler already gives Startup highest service priority and
  preserves accepted request ownership across cycles. Named Rescan projection
  can reuse the exact Startup action path.
- Lifecycle projection already clears topology readiness whenever Startup is
  not Ready. Named Rescan still needs an explicit service arm so diagnostics
  and terminal readiness cannot be confused with an ordinary cold start.

#### Missing Contract

- No stable rescan handle/status/result API exists.
- No whole-operation absolute Startup deadline exists; nested configured
  timeouts can otherwise exceed an explicit recovery deadline.
- Production reports cannot distinguish cold Startup from explicit rescan.
- Capability/docs currently state that rescan is planned.

<a id="09-27-ethercat-runtime-state-requests-research-existing-runtime-path-md"></a>

## 09-27-ethercat-runtime-state-requests/research/existing-runtime-path.md

[Original record](https://github.com/ra1mj/ESOP/tree/c7d43e51ed0a5e12107bad8f55267087801568fe/.trellis/tasks/archive/2026-09/09-27-ethercat-runtime-state-requests/research/existing-runtime-path.md) · SHA-256 `2eabc0d7deb9252993487acec7db1298a44382e81a8128898623020144cb3499`

### Existing Runtime State-Transition Path

#### Relevant Authorities

- `crates/esop-ethercat-core/src/al.rs`: one legal ESM step at a time with
  token/generation/deadline/WKC/status-code validation and optional AL-error
  acknowledgement.
- `crates/esop-ethercat-core/src/startup.rs`: retained slave/profile evidence,
  multi-step target traversal, Device Emulation policy, and private OpOnly
  SyncManager sequencing.
- `crates/esop-ethercat-core/src/slave.rs`: fixed-capacity retained records,
  legal next-state graph, requested-state fields, and observation updates.
- `crates/esop-ethercat-core/src/production_service.rs`: one fixed-priority
  service request retained across cycles after Domain/DC processing.
- `crates/esop-lifecycle-guard/src/ethercat.rs`: scheduler-selected service to
  lifecycle-gate projection.
- `crates/esop-ethercat-linux-port/tests/scheduled_domains.rs`: existing
  production-shaped ordering and delayed-response simulation patterns.

#### Findings

1. `AlTransitionController` deliberately performs one ESM step. A runtime API
   must orchestrate repeated verified steps instead of changing this authority.
2. `StartupController` already retains verified station, status, ESI transition
   timeouts, request timeout, Device Emulation, and OpOnly profile evidence, but
   its `start` method intentionally destroys that evidence and rescans.
3. Reusing Startup's whole FSM for one runtime state request would conflate
   topology readiness with a single-slave operation and would walk unrelated
   slaves after completion. A dedicated bounded controller is the smaller
   ownership boundary.
4. The production scheduler retains one request handle across cycles, rejects
   action mismatches, releases prepared requests after TX failure, and always
   runs Domain/DC transport before the selected service. The new service should
   extend these exact branches rather than introduce another scheduler.
5. Faulted configuration controllers remain selected because only Idle and
   Complete are inactive. State requests should use the same persistent
   fail-closed behavior because a timed-out AL write leaves the physical state
   uncertain.
6. Lifecycle currently gates Startup on topology and configuration/mailbox
   services on CoE readiness. A runtime ESM change is closest to topology
   readiness and should conservatively clear that gate while active/faulted.
7. Startup's OpOnly sequence is private and intertwined with its transition
   stage. This child must reject such profiles rather than duplicate or bypass
   the safety behavior.

#### Implementation Boundary

Create a standalone state-request FSM, a narrow Startup context/reconciliation
adapter, one optional production service binding, and one lifecycle projection.
Do not rescan, mutate PDO/DC configuration, add automatic policy, or claim
physical timing/interop evidence.

<a id="09-27-ethercat-single-slave-reconfigure-research-existing-configuration-reuse-md"></a>

## 09-27-ethercat-single-slave-reconfigure/research/existing-configuration-reuse.md

[Original record](https://github.com/ra1mj/ESOP/tree/c7d43e51ed0a5e12107bad8f55267087801568fe/.trellis/tasks/archive/2026-09/09-27-ethercat-single-slave-reconfigure/research/existing-configuration-reuse.md) · SHA-256 `b0b9a873179247baa745acbd259818c426e6c8121c99d8e3b163fe8e2fe84250`

### Existing Configuration Reuse Research

#### Retained startup evidence

`StartupController` retains verified slave records and profiles, mailbox/SII
evidence, SyncManager/FMMU register banks, selected DC reference, and full DC
topology. `start_mapping_for_position` already resolves per-position register
evidence. `SlaveRecord.configured` is the existing readiness bit, but requires
narrow target-only invalidation and commit APIs.

#### Reusable controllers

- `PdoConfigController` already configures one station from one bounded plan.
- `MappingConfigController` already clears, writes, and verifies one station's
  discovered SM/FMMU registers.
- `WatchdogController` can run a single-entry plan.
- `OpOnlySyncManagerController` can disable/read back OpOnly channels before a
  lower AL-state request.
- `StateRequestController` can drive PREOP directly with copied verified
  context after OpOnly channels are disabled.
- `DcClockController` and `DcSyncController` currently build full-slave plans;
  they need additive target-only start paths that still validate the full
  retained topology/reference context.

#### Product and scheduler boundaries

`StaticProductConfig` already builds a per-position PDO startup plan and full
watchdog/DC plans. The runtime owns the frozen `MappingTable`, so the targeted
builder must accept it rather than reconstruct process-image ownership.

The production scheduler runs Domain/DC first, then one service request, and
retains an accepted request across cycles. Reconfiguration should be a named
service that delegates to the current child controller instead of creating a
parallel transport or queue.

#### Decisions

- Stop at verified PREOP; no automatic SAFEOP/OP return.
- Invalidate only the target after complete admission and before any wire
  action.
- Leave the target unconfigured after any terminal fault; no automatic retry.
- Permit reading a non-target DC reference but never writing a non-target.
- Keep the public operation fixed-capacity and allocation-free.

<a id="09-27-sii-dc-descriptor-verification-research-sii-dc-layout-md"></a>

## 09-27-sii-dc-descriptor-verification/research/sii-dc-layout.md

[Original record](https://github.com/ra1mj/ESOP/tree/c7d43e51ed0a5e12107bad8f55267087801568fe/.trellis/tasks/archive/2026-09/09-27-sii-dc-descriptor-verification/research/sii-dc-layout.md) · SHA-256 `1b7512c0cf0803ae1e6e83a4dd463347b6cf718f46e877e6c3059993e4443526`

### SII DC layout research

#### Public behavior evidence

- EtherCAT SII category `0x003c` is represented as fixed 24-byte operation-mode
  entries by independent open-source EEPROM tooling.
- The observed field sequence is SYNC0 cycle time, SYNC0 shift, SYNC1 shift,
  SYNC1 cycle factor, AssignActivate, SYNC0 cycle factor, name index,
  description index and four reserved bytes.
- ESI exposes the corresponding values under `Device/Dc/OpMode`; the first
  mode is commonly treated as a default, but ESOP selects by explicit product
  name to avoid hidden ordering policy.
- The category does not carry a direct `CycleTimeSync1` value. ESOP's strict
  verifiable subset therefore accepts only zero direct content and preserves
  the factor represented online.

#### Sources consulted

- KickCAT `SIIBuilder` implementation/tests, including the 24-byte entry and
  generated name/description indices:
  `https://github.com/leducp/KickCAT/tree/089f8cad4e852ea3b9e1d0b407425abe7440e3f3`
- Synapticon `siitool` SII parser and ESI conversion behavior:
  `https://github.com/synapticon/siitool/tree/c46570adb1228eada44ae1f38bd61a81fab99c98`
- SOEM DC pulse programming behavior remains the reference for the subsequent
  start-time task, not for this parser:
  `https://github.com/OpenEtherCATsociety/SOEM/tree/88e8ed46efba7dfa7b94d08a512db25a33e3f8d5`

#### Independence boundary

These sources are used only to identify public protocol behavior and compare
interoperability expectations. ESOP code, type names, comments, validation
structure and tests are independently authored. No external source file,
structure definition, comment, or byte vector is copied into the project.

<a id="09-27-sii-fmmu-usage-verification-research-soem-sii-fmmu-md"></a>

## 09-27-sii-fmmu-usage-verification/research/soem-sii-fmmu.md

[Original record](https://github.com/ra1mj/ESOP/tree/c7d43e51ed0a5e12107bad8f55267087801568fe/.trellis/tasks/archive/2026-09/09-27-sii-fmmu-usage-verification/research/soem-sii-fmmu.md) · SHA-256 `796beaca4d8106734ae60c4ea526685e517d714262b306738aab94b6cf3b66e1`

### SOEM SII FMMU reference

Research date: 2026-09-27

Primary implementation inspected: OpenEtherCATsociety/SOEM, current default
branch clone at research time.

- `include/soem/ec_type.h` declares SII category 40 (`0x0028`) as FMMU.
- `src/ec_main.c::ecx_siiFMMU` reads the category word length, converts it to a
  byte count, and reads one function byte per FMMU index.
- `include/soem/ec_main.h` documents function values as `0=unused`,
  `1=outputs`, `2=inputs`, `3=SM status`.
- `src/ec_config.c` treats `0xff` as a marker that does not override the
  implementation's existing/default function value.

Source locations:

- https://github.com/OpenEtherCATsociety/SOEM/blob/master/src/ec_main.c
- https://github.com/OpenEtherCATsociety/SOEM/blob/master/include/soem/ec_main.h
- https://github.com/OpenEtherCATsociety/SOEM/blob/master/include/soem/ec_type.h

Implication for ESOP: category `0x0028` is capability/use metadata, not a source
of logical or physical address ranges. ESOP should verify ordered FMMU index
usage while continuing to derive addresses from its deterministic Domain/PDO
configuration.

<a id="09-27-topology-wide-dc-sync-programming-research-dc-sync-timing-md"></a>

## 09-27-topology-wide-dc-sync-programming/research/dc-sync-timing.md

[Original record](https://github.com/ra1mj/ESOP/tree/c7d43e51ed0a5e12107bad8f55267087801568fe/.trellis/tasks/archive/2026-09/09-27-topology-wide-dc-sync-programming/research/dc-sync-timing.md) · SHA-256 `3df9c0c23068f3489d406f62e5516ac8a950c99acda11bd4bbfd91e8759fc844`

### DC SYNC timing research

#### Repository evidence

- `crates/esop-ethercat-core/src/dc.rs` already defines the exact ESC
  registers, one-station disable/read/start/cycle/activate sequence, and a
  separate topology-wide offset/delay controller.
- `crates/esop-cfggen/src/esi.rs` retains direct SYNC0 cycle, signed SYNC0 and
  SYNC1 shifts, signed cycle factors, and the exact 16-bit `AssignActivate`.
- `crates/esop-product-config/src/lib.rs` freezes product base period, selected
  mode expectation, positions, stations, and reference-clock policy.
- Startup publishes immutable topology and verified online DC mode evidence
  before the PREOP configuration barrier.

#### Primary public behavior references

- SOEM `ethercatdc.c` computes the first start after `system_time + SyncDelay`,
  rounds to the next cycle, applies signed shift, writes start/cycle registers,
  and activates the unit. Source:
  `https://github.com/OpenEtherCATsociety/SOEM/blob/master/soem/ethercatdc.c`.
- IgH `fsm_slave_config.c` documents the generated timing relation used by
  EtherCAT masters: direct SYNC0 cycle takes precedence, otherwise positive
  and negative factors scale the application period; SYNC1 uses
  `T1 - cycle0 + shift1`. Source:
  `https://docs.etherlab.org/ethercat/1.6/doxygen/fsm__slave__config_8c_source.html`.
- IgH's public API states that the application time selects the phase of the
  slave clocks and that the first trigger is in the future. Source:
  `https://docs.etherlab.org/ethercat/1.6/doxygen/group__ApplicationInterface.html`.

#### ESOP decisions

1. Resolve timing once through a shared Rust contract used by cfggen and the
   `no_std` runtime boundary.
2. Reject inexact negative-factor division rather than silently changing a
   requested nanosecond period.
3. Preserve and finally write the complete `AssignActivate` word at `0x0980`.
4. Disable every slave before configuring any, and activate only after every
   cycle/start register has been accepted.
5. Align one common unshifted epoch to the LCM of all repeat periods, then add
   each verified SYNC0 shift.
6. Publish software evidence only after the complete batch; never describe
   accepted hardware writes as rolled back.
