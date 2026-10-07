# Historical Host Qualification Evidence and Research

These are dated September 2026 records preserved during repository cleanup.
They retain their original findings, citations and qualification limits.
Statements about missing features or future work describe that historical
snapshot, not the current implementation. Current source, backend specs and
the qualification manifests take precedence. No new HIL or release claim
is established by copying these records. Original bytes remain available
at the pinned source links; only heading levels and a local compiler path
are normalized below.

<a id="09-25-ebpf-irq-softirq-runtime-qualification-research-softirq-injection-md"></a>

## 09-25-ebpf-irq-softirq-runtime-qualification/research/softirq-injection.md

[Original record](https://github.com/ra1mj/ESOP/tree/c7d43e51ed0a5e12107bad8f55267087801568fe/.trellis/tasks/archive/2026-09/09-25-ebpf-irq-softirq-runtime-qualification/research/softirq-injection.md) · SHA-256 `61db4c4cd6e7960971b42ed91a798ffc10e1285ee7c10c68f16a55103bc198cc`

### Softirq injection research

#### Repository facts

- `bpf/esop_runtime.bpf.c` already measures complete hard-IRQ and softirq
  entry/exit duration, emits fixed evidence above independent thresholds, and
  records per-CPU samples/overruns.
- `crates/esop-ebpf-runtime/src/linux.rs` already attaches each pair atomically,
  decodes `SoftirqCpuTime`, aggregates statistics, and updates both thresholds.
- `crates/esop-ebpf-agent/src/lib.rs` currently merges every `HostIrqStorm`
  inside the time window without comparing interrupt class, CPU, or vector.
- The context ABI has an adjacent reserved `u16` and `u32` suitable for an
  optional CPU/vector filter without changing its 176-byte size.
- The shared interrupt emitter includes current TGID/TID. A bounded loopback
  burst handled inline should therefore retain the fixture PID/TID; those
  fields describe execution context, not interrupt ownership.

#### Kernel source evidence

- Linux kernel `kernel/softirq.c` defines vector 3 as `NET_RX` and calls
  `trace_softirq_entry(vec_nr)` immediately before `h->action()` and
  `trace_softirq_exit(vec_nr)` immediately after it. The tracepoint duration is
  therefore the complete vector action, not raise-to-run latency:
  https://github.com/torvalds/linux/blob/master/kernel/softirq.c
- The kernel tracepoint documentation states that `softirq_entry` and
  `softirq_exit` are paired to determine the handler routine:
  https://www.kernel.org/doc/html/v5.0/core-api/tracepoint.html
- Linux loopback transmit passes the skb to `__netif_rx`, which schedules the
  receive path consumed by `NET_RX` softirq:
  https://github.com/torvalds/linux/blob/master/drivers/net/loopback.c

#### Local injection experiment

Environment: 32 CPUs, no passwordless sudo, so the experiment used
`/proc/softirqs` rather than loading the BPF object.

- Connected loopback UDP with ordinary `sendmmsg` produced a target-CPU
  `NET_RX` delta equal to the message count (32 and 128 tested). It does not
  produce one isolated bounded handler and is unsuitable for exact evidence.
- Connected loopback UDP with one `sendmsg` and `UDP_SEGMENT=1200` produced a
  target-CPU `NET_RX` delta of exactly one for 8, 32, 40, 48, and 54 receive
  datagrams across repeated runs.
- 64 x 1200 bytes exceeded the UDP message-size limit. 54 x 1200 = 64,800 bytes
  stayed within the limit and maximized the tested bounded burst.
- Background `NET_RX` activity was visible on other CPUs. Exact BPF CPU/vector
  filtering is therefore required for a stable hosted qualification; global
  counts or a one-nanosecond global threshold would be noisy and could overflow
  the bounded incident evidence.

#### Decision

Use one 54-segment loopback UDP GSO send on a quiet allowed CPU, filter BPF
entry state to that CPU and vector 3, calibrate one handler duration in a
disposable runtime, then verify a fresh formal run above one eighth of the
measured calibration duration. Claim only the controlled hosted loopback
softirq chain.

<a id="09-25-ebpf-irq-softirq-runtime-qualification-result-md"></a>

## 09-25-ebpf-irq-softirq-runtime-qualification/result.md

[Original record](https://github.com/ra1mj/ESOP/tree/c7d43e51ed0a5e12107bad8f55267087801568fe/.trellis/tasks/archive/2026-09/09-25-ebpf-irq-softirq-runtime-qualification/result.md) · SHA-256 `6f5d93cf8cde3b3757ff93e7979d951fdf6e4fc85246b8c3d6a63ce003060f86`

### Result

- Completed: 2026-09-25
- Work commits: `73d05e7`, `275a816`
- GitHub Actions: `quality` run `36127109470` completed successfully for commit
  `275a816999f36e40701db764d8660f2498e89d86`.
- The dedicated `ebpf-softirq-runtime` job completed successfully in 1 minute
  41 seconds and uploaded `esop-ebpf-softirq-runtime-qualification`.
- The downloaded `ebpf_softirq_qualification.json` independently passed
  `scripts/validate-ebpf-softirq-qualification.py`.

#### Measured hosted evidence

- Target CPU: 0; quiet-window `NET_RX` delta: 0.
- Calibration duration: 44,867 ns; derived formal threshold: 5,608 ns.
- Formal duration: 44,446 ns.
- Formal statistics: one softirq sample, one overrun, one emitted event, zero
  hard-IRQ samples/overruns, and zero lost events.
- Correlation: Error `HostIrqStorm`, `ControlledStop`, confidence 70, followed
  by Degraded observation fault `0x45422001`.

#### Validation

- Local `make ci` passed.
- User-local Clang 14 was installed under `<local-clang14-install>`; the production
  BPF source compiled into a valid eBPF ELF object. Local privileged execution
  remained unavailable because the environment has neither root nor
  passwordless sudo, so the hosted Actions runner supplied the required real
  verifier/load/attach qualification.

<a id="09-25-ebpf-memcg-oom-runtime-qualification-research-linux-memcg-oom-md"></a>

## 09-25-ebpf-memcg-oom-runtime-qualification/research/linux-memcg-oom.md

[Original record](https://github.com/ra1mj/ESOP/tree/c7d43e51ed0a5e12107bad8f55267087801568fe/.trellis/tasks/archive/2026-09/09-25-ebpf-memcg-oom-runtime-qualification/research/linux-memcg-oom.md) · SHA-256 `e72a6ceecbb8f1a7e6470894ba2ffe4823c5f8aa58dc8d2d29a3672d9f612ef2`

### Linux memcg OOM qualification research

Date: 2026-09-25

#### Primary sources

- Linux cgroup v2 documentation:
  `https://docs.kernel.org/admin-guide/cgroup-v2.html`
- Linux OOM tracepoint definition:
  `https://github.com/torvalds/linux/blob/master/include/trace/events/oom.h`
- Linux OOM victim implementation:
  `https://github.com/torvalds/linux/blob/master/mm/oom_kill.c`

#### Findings used by the design

- `memory.max` is the cgroup v2 hard memory limit. If usage reaches the limit
  and reclaim cannot reduce it, the OOM killer is invoked inside the cgroup.
- A cgroup-local OOM does not kill tasks outside that cgroup.
- `memory.events.local` exposes local `max`, `oom`, `oom_kill`, and
  `oom_group_kill` counters. `oom_kill` counts killed processes; `oom` may
  advance more than once around a single kill and is used only as a lower
  bound.
- `memory.oom.group=0` keeps the default per-victim behavior. The fixture still
  uses a single-process leaf to avoid ambiguous group kills.
- `TRACE_EVENT(mark_victim)` stores `task->pid` plus memory/accounting fields.
  The current ESOP ABI deliberately exports only the raw PID and does not claim
  a separate TGID, namespace, or cgroup identity.
- The kernel sends `SIGKILL` to the selected victim and calls
  `mark_oom_victim`; the qualification therefore requires both the tracepoint
  event and a `SIGKILL` child wait status.

<a id="09-25-ebpf-memcg-oom-runtime-qualification-result-md"></a>

## 09-25-ebpf-memcg-oom-runtime-qualification/result.md

[Original record](https://github.com/ra1mj/ESOP/tree/c7d43e51ed0a5e12107bad8f55267087801568fe/.trellis/tasks/archive/2026-09/09-25-ebpf-memcg-oom-runtime-qualification/result.md) · SHA-256 `f4b60f3b2b26ecff580ae0f697d4ac6a65de252414e73694624f1bc458e07800`

### Result: eBPF memcg OOM runtime qualification

Date: 2026-09-25

#### Delivered

- Added a Linux-only controlled cgroup-v2 OOM qualification fixture that
  prewarms a single-threaded child, moves only that PID into a unique leaf,
  applies a 32 MiB hard memory limit, injects a bounded 128 MiB anonymous
  mapping, and requires `SIGKILL` plus successful leaf cleanup.
- Loaded the production CO-RE object with only `ATTACH_OOM_KILL` enabled and
  required, then verified the exact victim PID through fixed evidence,
  Critical `HostOom` incident correlation, and Healthy-to-Failed observation
  projection.
- Added a closed-schema report validator, six regression tests, Make targets,
  a dedicated privileged GitHub Actions job, artifact upload, capability
  manifest evidence, PRD/design text, README guidance, and backend quality
  contracts.
- Preserved the narrow claim: this qualifies one hosted single-process memcg
  victim-PID chain, not global OOM, victim TGID, namespace/cgroup attribution,
  trigger/root cause, containers, restart policy, production kernels, or
  overhead/WCET.

#### Verification

- Work commit: `560e85dda668fd930f9ab2a93653ca573e36afba`
- GitHub Actions run: `36134613286`
- Run URL: `https://github.com/ra1mj/ESOP/actions/runs/36134613286`
- All 10 jobs passed, including `ebpf-oom-runtime` and the full Rust/Zenoh
  quality job.
- Downloaded artifact:
  `esop-ebpf-oom-runtime-qualification/ebpf_oom_qualification.json`
- The downloaded report independently passed
  `scripts/validate-ebpf-oom-qualification.py`.
- Local `make ci`, focused Rust check/Clippy, six validator tests, BPF syntax,
  capability-manifest validation, shell/Python syntax, formatting, and
  `git diff --check` passed.

#### Hosted measurements

- cgroup v2 memory controller enabled; leaf membership changed from exactly
  one child to zero and cleanup succeeded.
- `memory.max=33554432`, mapping `134217728`, page size `4096`, swap limit `0`,
  and group OOM disabled.
- Child PID/TID `7956` terminated with signal `9` (`SIGKILL`).
- Local counter deltas: `max=35`, `oom=1`, `oom_kill=1`,
  `oom_group_kill=0`.
- Attach masks were exactly `16` (`ATTACH_OOM_KILL`).
- BPF/poll results: one emitted OOM record, one incident, zero process exits,
  zero loss, zero malformed/rejected records, and zero dropped incidents.
- Incident policy: Critical `HostOom`, `LatchFault`, confidence `100`, no cycle
  or transition correlation.
- Final observation: Failed, one incident, zero loss, fault `0x45422002`,
  heartbeat sequence `2`.

#### Residual limits

The result does not establish victim TGID, PID namespace or cgroup identity,
allocation trigger/cause, global/system OOM behavior, delegated container
behavior, swap workloads, component restart supervision, production target
kernels, sustained pressure, overhead, WCET, or long-duration HIL.

<a id="09-25-ebpf-network-drop-runtime-qualification-result-md"></a>

## 09-25-ebpf-network-drop-runtime-qualification/result.md

[Original record](https://github.com/ra1mj/ESOP/tree/c7d43e51ed0a5e12107bad8f55267087801568fe/.trellis/tasks/archive/2026-09/09-25-ebpf-network-drop-runtime-qualification/result.md) · SHA-256 `b7f2d402435020f9f228ac2c550f87caf6854b24604fb1566cf78b69bdc1cb84`

### Result: eBPF network-drop runtime qualification

Date: 2026-09-25

#### Delivered

- Added a Linux-only qualification fixture that creates an isolated veth pair,
  disables per-link IPv6, pins itself to one inherited CPU, and injects complete
  protocol-zero AF_PACKET Ethernet frames without registering a receive handler.
- Loaded the production CO-RE object with only `ATTACH_NETWORK_DROP` enabled and
  required, qualified protocol and receive-ifindex controls, and proved the
  four-drop threshold through independent link counters and exact BPF statistics.
- Verified one fixed `KernelNetwork/NetworkDrop/Error` evidence item, one Error
  `HostNicDrop`/`ControlledStop` incident at confidence 75, and a
  Healthy-to-Degraded observer-health transition.
- Added a closed-schema validator, six regression tests, Make targets, a
  dedicated privileged GitHub Actions job, artifact upload, capability evidence,
  PRD/design/README text, and backend quality contracts.
- Corrected the vendored `kfree_skb.reason` BTF model to preserve the target
  `enum skb_drop_reason` kind and added compile-time kind/width assertions. This
  prevents Aya from poisoning an impossible `FIELD_SIGNED` relocation as helper
  `0x0bad2310` during target-kernel load.
- Preserved the narrow claim: this qualifies one hosted virtual-veth
  unhandled-EtherType receive-drop chain, not physical NIC, driver, NAPI, XDP,
  qdisc, queue pressure, congestion, real EtherCAT device, production-kernel,
  overhead, WCET, or long-duration HIL behavior.

#### Verification

- Work commits: `df732696e3e99343c82f1b30b45a11594803fc0c`,
  `4a45b9f92621f28f891361cd737f62d756d47809`, and
  `5a5294b684641961db1397cfa10989d508d3a563`.
- GitHub Actions run: `36142735199`.
- Run URL: `https://github.com/ra1mj/ESOP/actions/runs/36142735199`.
- All 11 jobs passed, including `ebpf-network-drop-runtime`, all existing
  privileged eBPF qualifications, CO-RE object build, and the full Rust/Zenoh
  quality job.
- Downloaded artifact:
  `esop-ebpf-network-drop-runtime-qualification/ebpf_network_drop_qualification.json`.
- The downloaded report independently passed
  `scripts/validate-ebpf-network-drop-qualification.py` and has SHA-256
  `d54382f7b70805734a662ab68b9438d92c713b4c951784e2cae17c786c503fe0`.
- Local `make ci`, focused Rust tests, six validator tests, BPF syntax, real
  Clang BPF object generation/BTF inspection, capability-manifest validation,
  formatting, Clippy, release/no-std builds, and `git diff --check` passed.

#### Hosted measurements

- Interfaces `esotx1f4e`/`esorx1f4e` used ifindexes 6/5, distinct unicast MACs,
  MTU 1500, link-up state, and per-link IPv6 disablement. The fixture was pinned
  to CPU 0 and used packet-socket protocol zero.
- Controls sent one forward `0x88a5` frame and one reverse `0x88a4` frame. Each
  expected independent receive-drop counter advanced once while matching BPF
  counters, records, incidents, loss, and health remained zero/healthy.
- Four 64-byte forward EtherCAT frames changed RX drops from 1 to 5, for an
  exact formal delta of 4. TX drops stayed at 1, for an exact formal delta of 0.
- Attach masks were exactly 32 (`ATTACH_NETWORK_DROP`), threshold 4, and window
  1,000,000,000 ns.
- BPF/poll results were `network_drops=4`, `network_unattributed=0`, one threshold
  event, one emitted/seen record, one incident, zero loss, zero malformed or
  rejected evidence, and zero dropped incidents.
- Evidence used CPU 0, RX ifindex 5, detail 74, duration 13,756 ns, value/
  threshold/count 4, cycle 42, and transition 9.
- The incident was Error `HostNicDrop`, action `ControlledStop`, confidence 75.
  Observation changed from Healthy heartbeat 1 to Degraded heartbeat 2 with
  fault `0x45422001`.
- Runtime detach, packet-socket close, link deletion, cleanup verification, and
  affinity restoration all succeeded before the qualified report was published.

#### Debug retrospective

- Category: implicit cross-layer BTF assumption plus a target-load test gap.
- Initial source/object checks did not exercise Aya's relocation against target
  kernel BTF. The first hosted run exposed a poisoned field relocation. Changing
  signedness alone left local `INT` versus target `ENUM` incompatible; preserving
  the named enum kind resolved the relocation.
- Prevention now combines compile-time kind/width assertions, object BTF
  inspection, the backend quality contract, and the mandatory privileged
  target-kernel verifier/load job.

#### Residual limits

The result does not establish physical-NIC or driver drop behavior, NAPI/XDP/
qdisc attribution, queue pressure or congestion cause, checksum/link failure,
real EtherCAT-device behavior, production target-kernel portability, overhead,
WCET, sustained load, or long-duration HIL stability.

<a id="09-25-ebpf-observability-degradation-qualification-result-md"></a>

## 09-25-ebpf-observability-degradation-qualification/result.md

[Original record](https://github.com/ra1mj/ESOP/tree/c7d43e51ed0a5e12107bad8f55267087801568fe/.trellis/tasks/archive/2026-09/09-25-ebpf-observability-degradation-qualification/result.md) · SHA-256 `71d4a640df252324f4d5c0763d9898c1cedae4f1cc125a0e243487c4e24a56c9`

### Result: eBPF observability degradation runtime qualification

Date: 2026-09-25

#### Delivered

- Added a dedicated event-loss fault (`0x45421004`) and made positive loss
  sticky within one agent epoch. A healthy capability refresh no longer hides
  known loss, while hard verifier/permission failure retains Failed precedence.
- Reset attach, loss, incident, fault, heartbeat, and event-time state at agent
  restart so a new epoch begins conservatively and can recover only after a new
  runtime publishes a complete capability snapshot.
- Added a Linux/x86_64 qualification fixture that loads the production CO-RE
  object with only `exceptions:page_fault_user`, keeps the shipped 4 MiB
  ringbuf, and fills it through bounded 64 MiB anonymous first-write batches
  without polling the event channel.
- Proved exact kernel-loss projection into `RuntimeAgent`, same-epoch Degraded
  stickiness, runtime detach, epoch-2 reset, production-object reload, Healthy
  recovery, and accepted new-epoch evidence with zero new loss or rejection.
- Added a closed-schema validator, six mutation tests, Make targets, a dedicated
  privileged GitHub Actions job and artifact, cross-layer MLG regression, and
  synchronized README/PRD/runtime/capability/Trellis quality contracts.
- Preserved the claim boundary: missing BTF/ringbuf/required attach and
  verifier/permission states are deterministic policy tests, not hosted fault
  injection; production pressure, target kernels, overhead/WCET, and long
  duration remain open.

#### Verification

- Work commit: `9db984178f79157d177d2688e665b902d434d204`.
- GitHub Actions run: `36151603726` (`quality`, run 133).
- Run URL: `https://github.com/ra1mj/ESOP/actions/runs/36151603726`.
- All 12 jobs passed, including `ebpf-observability-degradation-runtime`, all
  existing privileged eBPF qualifications, CO-RE build, and Rust/Zenoh quality.
- Artifact: `esop-ebpf-observability-degradation-qualification`, artifact ID
  `10871913161`.
- Downloaded ZIP SHA-256:
  `0b14673d0de0636304e5bcbb47e063902d2e56ddd68fd8b48ab0cded91e28c52`;
  this exactly matched the GitHub artifact digest.
- Extracted report SHA-256:
  `341941b35f30dfd1f6780ce2c32b03fa96dc2facdf403b5eed89e9178c3da3fc`.
- The downloaded report independently passed
  `scripts/validate-ebpf-observability-degradation-qualification.py`.
- Local `make ci`, formatting, workspace tests, Clippy, release/no-std builds,
  BPF C syntax, capability validation, Python mutation/compile tests, shell
  syntax, example build, and `git diff --check` passed.
- The local privileged target could not run because this workstation has no
  Clang executable, root session, or passwordless sudo. No local runtime claim
  was made; the dedicated hosted runner supplied the privileged evidence.

#### Hosted measurements

| Field | Value |
| --- | ---: |
| architecture | `x86_64` |
| production ringbuf | `4,194,304 bytes` |
| chunk size | `67,108,864 bytes` |
| saturation batches | `3 / 8` |
| pages touched | `49,152` |
| successful emitted events | `40,329` |
| kernel lost events | `8,823` |
| page faults | `49,152` |
| saturation poll records | `1` |
| projected new loss | `8,823` |
| loss/reapplied state | `Degraded / Degraded` |
| loss fault | `0x45421004` |
| second agent epoch | `2` |
| Restarting attach/loss | `0 / 0` |
| recovered/final state | `Healthy / Healthy` |
| recovery records | `64` |
| recovery loss/rejection | `0 / 0` |
| both runtimes detached / cleanup | `yes / yes` |

The first runtime's loss count was projected exactly once and remained visible
after the epoch-1 capability refresh. Restart cleared every epoch-local health
field before runtime 2 loaded. Runtime 2 accepted new evidence, retained zero
loss, and published the report only after both runtimes detached.

#### Residual limits

This result qualifies one hosted x86_64 page-fault-driven saturation and
unload/reload recovery path. It does not establish actual missing-BTF,
missing-ringbuf, missing-attach, verifier-rejection, or permission-denial host
behavior; exact ringbuf record capacity; production event pressure; portable
target-kernel behavior; CPU/RAM overhead; WCET; ringbuf watermarks; sustained
load; or long-duration HIL stability.

<a id="09-25-ebpf-page-fault-runtime-qualification-research-page-fault-tracepoint-md"></a>

## 09-25-ebpf-page-fault-runtime-qualification/research/page-fault-tracepoint.md

[Original record](https://github.com/ra1mj/ESOP/tree/c7d43e51ed0a5e12107bad8f55267087801568fe/.trellis/tasks/archive/2026-09/09-25-ebpf-page-fault-runtime-qualification/research/page-fault-tracepoint.md) · SHA-256 `16f0187ff31e60d9e4792be653b8c5f8ab00d9102a4d5a63541e8dc4ad1150e0`

### Page-fault tracepoint research

#### Production path already present

- `bpf/esop_runtime.bpf.c` attaches `esop_page_fault_user` to
  `exceptions:page_fault_user`, filters by tracked TGID, increments
  `page_faults`, aggregates in the fixed `ESOP_PAGE_FAULTS` LRU map keyed by
  CPU/TGID, and emits only the first threshold crossing.
- The emitted detail is the saturated architecture `error_code`; the existing
  fixed evidence record does not carry tracepoint address or instruction
  pointer.
- `crates/esop-ebpf-agent/src/lib.rs` requires count threshold plus cycle risk
  and maps qualifying evidence to Warning `HostPageFault`,
  `DegradeHostObservation`, confidence 65.

#### Kernel source facts

- Linux `include/trace/events/exceptions.h` declares a shared exception event
  with `address`, `ip`, and `error_code`, then defines `page_fault_user` from
  that class:
  https://github.com/torvalds/linux/blob/master/include/trace/events/exceptions.h
- Linux trace-event generation materializes the raw event class fields in the
  `trace_event_raw_*` record consumed by BPF:
  https://github.com/torvalds/linux/blob/master/include/trace/trace_events.h
- The x86 page-fault handler treats `error_code` as the hardware reason bits;
  a user write to a non-present anonymous page is expected to carry user and
  write bits without the protection/present bit:
  https://github.com/torvalds/linux/blob/master/arch/x86/mm/fault.c

#### Qualification decision

- Use an exec child that is fully prepared before BPF load, then track only its
  TGID. This isolates the parent runtime/allocator from page-fault statistics.
- Pin the child to one CPU because production aggregation is per CPU/TGID.
- First-write 16 untouched anonymous pages and independently require
  `ru_minflt` delta 16. The BPF record remains an exception-count observation;
  the supporting process counter must not be used to expand production event
  semantics.
- Scope the exact error detail to hosted x86_64. Architecture portability and
  major/minor attribution remain open.

<a id="09-25-ebpf-page-fault-runtime-qualification-result-md"></a>

## 09-25-ebpf-page-fault-runtime-qualification/result.md

[Original record](https://github.com/ra1mj/ESOP/tree/c7d43e51ed0a5e12107bad8f55267087801568fe/.trellis/tasks/archive/2026-09/09-25-ebpf-page-fault-runtime-qualification/result.md) · SHA-256 `45dee0bac2538e9c7b85f204eba39532e0e682ee77ac552ea79f6c3f3919db41`

### Result: eBPF page-fault runtime qualification

#### Outcome

Completed on 2026-09-25 in work commit
`ba26760e7750a4cd149b3e9021a0007d5eef470c`.

The task added a Linux-only controlled page-fault fixture, closed-schema report
validator and regression tests, Make integration, a dedicated privileged
GitHub Actions job, capability evidence, and synchronized PRD/runtime/quality
documentation. Production BPF programs, maps, fixed event/context ABI,
runtime APIs, and incident policy were not changed.

#### Hosted qualification evidence

- GitHub Actions run: `36130887979` (`quality`), completed successfully with
  all nine jobs passing.
- Dedicated job: `ebpf-page-fault-runtime`, completed in 2m42s and uploaded
  `esop-ebpf-page-fault-runtime-qualification`.
- Downloaded report:
  `build/ci-page-fault-36130887979/ebpf_page_fault_qualification.json`.
- The downloaded artifact independently passed
  `scripts/validate-ebpf-page-fault-qualification.py`.

Measured report values:

| Field | Value |
| --- | ---: |
| architecture | `x86_64` |
| tracked child PID | `7822` |
| target CPU | `0` |
| page size / page count | `4096` / `16` |
| child `ru_minflt` delta | `16` |
| BPF page faults | `16` |
| threshold events / emitted records / incidents | `1` / `1` / `1` |
| lost events | `0` |
| x86 error-code detail | `6` |
| count-window duration | `44,827 ns` |
| incident | Warning `HostPageFault`, confidence `65` |
| action | `DegradeHostObservation` |
| final observation | Degraded, fault `0x45422001`, heartbeat sequence `2` |

#### Verification

- `make ci` passed locally.
- Focused format, Clippy, Rust example build, Python unit/compile, shell syntax,
  BPF C syntax, capability manifest, CO-RE object build, and `git diff --check`
  passed.
- The local privileged target rebuilt the CO-RE object and fixture, then
  correctly refused to claim qualification because the workstation has neither
  root execution nor passwordless sudo.
- The hosted privileged target completed real verifier/load, required
  `exceptions:page_fault_user` attach, exact injection, ringbuf decode,
  statistics, incident correlation, health projection, validation, and artifact
  upload.

#### Preserved limits

This result qualifies one controlled hosted x86_64 anonymous-page first-write
count-window chain only. It does not expose fault address/IP, classify
major/minor outcome from BPF, measure fault-handler duration, or qualify memory
pressure, swap/storage, natural workload causality, product thresholds,
production kernels, overhead, WCET, or long-running HIL.

<a id="09-25-ebpf-softirq-qualification-gating-research-ci-failure-analysis-md"></a>

## 09-25-ebpf-softirq-qualification-gating/research/ci-failure-analysis.md

[Original record](https://github.com/ra1mj/ESOP/tree/c7d43e51ed0a5e12107bad8f55267087801568fe/.trellis/tasks/archive/2026-09/09-25-ebpf-softirq-qualification-gating/research/ci-failure-analysis.md) · SHA-256 `ad56cbbabf58795360e230195a1276a6de163d2b6879966eb9b5cc95004d1500`

### Bug Analysis: softirq qualification admitted ambient NET_RX work

#### 1. Root Cause Category

- **Category**: E - Implicit Assumption
- **Specific cause**: the qualifier assumed that an exact CPU/vector predicate
  plus a prior quiet sample isolated the controlled injection. That predicate
  was active from runtime load through ring-buffer polling, so it provided
  spatial filtering but not temporal isolation. GitHub Actions run
  `36154809386` observed two records/incidents even though the injection's
  target-CPU `/proc/softirqs` delta had already passed the exact-one check.

#### 2. Evidence and confidence

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

#### 3. Prevention Mechanisms

| Priority | Mechanism | Specific action | Status |
|----------|-----------|-----------------|--------|
| P0 | Architecture | Load with an impossible exact vector, open vector 3 only around send/counter sampling, then close before receive/poll | Done |
| P0 | Test contract | Preserve exact one record/incident/statistics and add closed/open/closed report fields with mutation tests | Done |
| P1 | Documentation | Record that spatial selectors do not establish temporal isolation in backend quality guidance | Done |
| P1 | Remote integration | Require the dedicated privileged hosted job and independently validated artifact | Done |

#### 4. Systematic Expansion

- **Similar issues**: a future hard-IRQ fixture cannot treat CPU/IRQ identity as
  temporal isolation; it needs a counter-bounded gate or a uniquely attributable
  producer. Other current qualifications use stronger identities such as exact
  TID, process TGID, unique veth ifindex/protocol, or explicit user markers.
- **Design improvement**: keep production observation broad, but make controlled
  qualification fixtures own a short admission window and publish its state.
- **Process improvement**: when a test claims exact event counts, compare the
  selector's lifetime with the stimulus lifetime, not only its value domain.

#### 5. Knowledge Capture

- [x] Update `.trellis/spec/backend/quality-guidelines.md` with the temporal-gate
      contract and wrong/correct examples.
- [x] Update the report schema, validator, and mutation tests.
- [x] Update README, product PRD, observability design, and capability manifest.
- [x] Capture hosted run and artifact measurements in `result.md` after push.

<a id="09-25-ebpf-softirq-qualification-gating-result-md"></a>

## 09-25-ebpf-softirq-qualification-gating/result.md

[Original record](https://github.com/ra1mj/ESOP/tree/c7d43e51ed0a5e12107bad8f55267087801568fe/.trellis/tasks/archive/2026-09/09-25-ebpf-softirq-qualification-gating/result.md) · SHA-256 `5adf0b07f3a5e3115ba7f2fbda66a5cbfedc850c8fab2b9bfa11622a6af52d97`

### Result: eBPF softirq qualification temporal gating

Date: 2026-09-25

#### Delivered

- Changed the hosted softirq qualification fixture so exact CPU/vector
  matching is treated as spatial filtering only. Each calibration and formal
  runtime now starts on qualification-only closed vector `u32::MAX - 1`, opens
  `NET_RX` vector 3 only across the counter-bounded GSO send, and closes before
  socket draining or ring-buffer polling.
- Kept the production runtime API, kernel-context ABI, BPF maps, attach masks,
  all-vector sentinel, evidence ABI, and incident policy unchanged by reusing
  `BpfRuntime::update_interrupt_filter`.
- Preserved the strict exact-one contract for target-CPU `NET_RX`, records,
  incidents, samples, overruns, and emitted events with zero loss. No retry or
  relaxed count was introduced.
- Added seven closed/open/closed report fields, strict schema validation,
  mutation tests, synchronized README/PRD/runtime/capability documentation, and
  a durable backend quality rule covering temporal qualification isolation.

#### Verification

- Work commit: `d2b7eeb9641fec87444f2aabc2ae6ba7855ae414`.
- GitHub Actions `quality` run `36160745503` (run 135) completed successfully;
  all 12 jobs passed, including dedicated `ebpf-softirq-runtime` job
  `108156404240`, CO-RE build, every other privileged eBPF qualification, and
  the complete Rust/Zenoh gate.
- Artifact `esop-ebpf-softirq-runtime-qualification`, artifact ID
  `10876025194`, was downloaded independently.
- ZIP SHA-256 `ca16d3489eddffd05666a870d4efc6a9c0195b817bc477b70e2a8ee4135261e7`
  exactly matched the GitHub artifact digest.
- Extracted report SHA-256:
  `bc8836fc0d39e32e81089de80cf918d9d32bb6673d1643322e57a456ec832eab`.
- The downloaded report independently passed
  `scripts/validate-ebpf-softirq-qualification.py`.
- Local `make ci`, formatting, focused Rust tests/build/Clippy, eight Python
  validator tests, Python/shell/BPF syntax, capability validation, release and
  no-std checks, and `git diff --check` passed.

#### Hosted measurements

| Field | Value |
| --- | ---: |
| target CPU | `0` |
| quiet target-CPU `NET_RX` delta | `0` |
| closed vector | `4294967294` |
| calibration gate | `4294967294 -> 3 -> 4294967294` |
| formal gate | `4294967294 -> 3 -> 4294967294` |
| calibration/formal `NET_RX` delta | `1 / 1` |
| calibration duration | `70,390 ns` |
| derived formal threshold | `8,798 ns` |
| formal duration | `40,395 ns` |
| sent bytes / received datagrams per phase | `64,800 / 54` |
| formal records / incidents | `1 / 1` |
| formal softirq samples / overruns / emitted | `1 / 1 / 1` |
| hard-IRQ samples / overruns | `0 / 0` |
| lost / newly reported lost events | `0 / 0` |
| malformed / rejected / dropped incidents | `0 / 0 / 0` |
| observation state | `Healthy -> Degraded` |
| observation fault | `0x45422001` |

#### Root cause and residual limits

The failed predecessor run admitted unrelated vector-3 activity because the
filter remained open from load through polling. The exact `/proc/softirqs`
delta proved the controlled send itself still produced one target action, so
the correction narrowed the admission lifetime instead of weakening evidence.

This qualifies one controlled hosted loopback `NET_RX` softirq-duration chain.
It does not establish hard-IRQ behavior, physical NIC/driver/NAPI behavior,
production interrupt budgets, sustained pressure, causal task ownership,
target-kernel portability, overhead/WCET, or long-duration HIL stability.
