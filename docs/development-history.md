# Development History

Completed task plans and personal journals are local working records. They
are excluded from the current repository tree; source, tests, fixtures,
shared workflow tooling, current specs and active plans remain versioned.

The complete archived planning record is preserved in Git at commit
`c7d43e51ed0a5e12107bad8f55267087801568fe`. Each link below is pinned to that immutable snapshot,
including decisions and completion checklists that are not current product
requirements. In a full clone, retrieve any original with
`git show <commit>:<original-path>`; no history rewriting is involved.

The 24 dedicated research, acceptance and run-result records are also
retained in the current tree, with their source hashes and links:

- [EtherCAT design research](history/ethercat-research.md)
- [Host qualification evidence](../qualification/host-evidence-history.md)
- [Software acceptance evidence](../qualification/software-evidence-history.md)

Current contracts live in the [backend spec index](../.trellis/spec/backend/index.md),
[EtherCAT requirements](ethercat-master-requirements.md),
[diagnostic CLI contract](esop-runtime-diagnostics-cli.md), and
[qualification limitations](../qualification/KNOWN_LIMITATIONS.md).

## Completed work

| Archived task | Original planning and completion record |
| --- | --- |
| Bootstrap Guidelines | [00-bootstrap-guidelines](https://github.com/ra1mj/ESOP/tree/c7d43e51ed0a5e12107bad8f55267087801568fe/.trellis/tasks/archive/2026-09/00-bootstrap-guidelines) |
| CiA 402 cyclic PDO binding | [09-04-cia402-cyclic-pdo](https://github.com/ra1mj/ESOP/tree/c7d43e51ed0a5e12107bad8f55267087801568fe/.trellis/tasks/archive/2026-09/09-04-cia402-cyclic-pdo) |
| eBPF IRQ and softirq attribution | [09-21-ebpf-irq-softirq-attribution](https://github.com/ra1mj/ESOP/tree/c7d43e51ed0a5e12107bad8f55267087801568fe/.trellis/tasks/archive/2026-09/09-21-ebpf-irq-softirq-attribution) |
| R2 Qualification Evidence Gate | [09-21-r2-qualification-evidence](https://github.com/ra1mj/ESOP/tree/c7d43e51ed0a5e12107bad8f55267087801568fe/.trellis/tasks/archive/2026-09/09-21-r2-qualification-evidence) |
| eBPF CPU frequency limit attribution | [09-25-ebpf-cpu-frequency-limit-attribution](https://github.com/ra1mj/ESOP/tree/c7d43e51ed0a5e12107bad8f55267087801568fe/.trellis/tasks/archive/2026-09/09-25-ebpf-cpu-frequency-limit-attribution) |
| eBPF CPU migration attribution | [09-25-ebpf-cpu-migration-attribution](https://github.com/ra1mj/ESOP/tree/c7d43e51ed0a5e12107bad8f55267087801568fe/.trellis/tasks/archive/2026-09/09-25-ebpf-cpu-migration-attribution) |
| eBPF gateway callback stall attribution | [09-25-ebpf-gateway-callback-stall-attribution](https://github.com/ra1mj/ESOP/tree/c7d43e51ed0a5e12107bad8f55267087801568fe/.trellis/tasks/archive/2026-09/09-25-ebpf-gateway-callback-stall-attribution) |
| eBPF gateway runtime qualification | [09-25-ebpf-gateway-runtime-qualification](https://github.com/ra1mj/ESOP/tree/c7d43e51ed0a5e12107bad8f55267087801568fe/.trellis/tasks/archive/2026-09/09-25-ebpf-gateway-runtime-qualification) |
| eBPF gateway stall attribution | [09-25-ebpf-gateway-stall-attribution](https://github.com/ra1mj/ESOP/tree/c7d43e51ed0a5e12107bad8f55267087801568fe/.trellis/tasks/archive/2026-09/09-25-ebpf-gateway-stall-attribution) |
| eBPF IRQ/softirq runtime qualification | [09-25-ebpf-irq-softirq-runtime-qualification](https://github.com/ra1mj/ESOP/tree/c7d43e51ed0a5e12107bad8f55267087801568fe/.trellis/tasks/archive/2026-09/09-25-ebpf-irq-softirq-runtime-qualification) |
| eBPF Linux raw port stall attribution | [09-25-ebpf-linux-raw-port-stall-attribution](https://github.com/ra1mj/ESOP/tree/c7d43e51ed0a5e12107bad8f55267087801568fe/.trellis/tasks/archive/2026-09/09-25-ebpf-linux-raw-port-stall-attribution) |
| eBPF memcg OOM runtime qualification | [09-25-ebpf-memcg-oom-runtime-qualification](https://github.com/ra1mj/ESOP/tree/c7d43e51ed0a5e12107bad8f55267087801568fe/.trellis/tasks/archive/2026-09/09-25-ebpf-memcg-oom-runtime-qualification) |
| eBPF EtherCAT network drop attribution | [09-25-ebpf-network-drop-attribution](https://github.com/ra1mj/ESOP/tree/c7d43e51ed0a5e12107bad8f55267087801568fe/.trellis/tasks/archive/2026-09/09-25-ebpf-network-drop-attribution) |
| eBPF network drop runtime qualification | [09-25-ebpf-network-drop-runtime-qualification](https://github.com/ra1mj/ESOP/tree/c7d43e51ed0a5e12107bad8f55267087801568fe/.trellis/tasks/archive/2026-09/09-25-ebpf-network-drop-runtime-qualification) |
| Qualify eBPF observability degradation | [09-25-ebpf-observability-degradation-qualification](https://github.com/ra1mj/ESOP/tree/c7d43e51ed0a5e12107bad8f55267087801568fe/.trellis/tasks/archive/2026-09/09-25-ebpf-observability-degradation-qualification) |
| eBPF page fault window attribution | [09-25-ebpf-page-fault-attribution](https://github.com/ra1mj/ESOP/tree/c7d43e51ed0a5e12107bad8f55267087801568fe/.trellis/tasks/archive/2026-09/09-25-ebpf-page-fault-attribution) |
| eBPF page-fault runtime qualification | [09-25-ebpf-page-fault-runtime-qualification](https://github.com/ra1mj/ESOP/tree/c7d43e51ed0a5e12107bad8f55267087801568fe/.trellis/tasks/archive/2026-09/09-25-ebpf-page-fault-runtime-qualification) |
| eBPF process-exit runtime qualification | [09-25-ebpf-process-exit-runtime-qualification](https://github.com/ra1mj/ESOP/tree/c7d43e51ed0a5e12107bad8f55267087801568fe/.trellis/tasks/archive/2026-09/09-25-ebpf-process-exit-runtime-qualification) |
| eBPF process and OOM lifecycle attribution | [09-25-ebpf-process-oom-attribution](https://github.com/ra1mj/ESOP/tree/c7d43e51ed0a5e12107bad8f55267087801568fe/.trellis/tasks/archive/2026-09/09-25-ebpf-process-oom-attribution) |
| eBPF raw-port runtime qualification | [09-25-ebpf-raw-port-runtime-qualification](https://github.com/ra1mj/ESOP/tree/c7d43e51ed0a5e12107bad8f55267087801568fe/.trellis/tasks/archive/2026-09/09-25-ebpf-raw-port-runtime-qualification) |
| eBPF scheduler migration runtime qualification | [09-25-ebpf-scheduler-migration-runtime-qualification](https://github.com/ra1mj/ESOP/tree/c7d43e51ed0a5e12107bad8f55267087801568fe/.trellis/tasks/archive/2026-09/09-25-ebpf-scheduler-migration-runtime-qualification) |
| eBPF scheduler runqueue runtime qualification | [09-25-ebpf-scheduler-runqueue-runtime-qualification](https://github.com/ra1mj/ESOP/tree/c7d43e51ed0a5e12107bad8f55267087801568fe/.trellis/tasks/archive/2026-09/09-25-ebpf-scheduler-runqueue-runtime-qualification) |
| Stabilize eBPF softirq qualification gating | [09-25-ebpf-softirq-qualification-gating](https://github.com/ra1mj/ESOP/tree/c7d43e51ed0a5e12107bad8f55267087801568fe/.trellis/tasks/archive/2026-09/09-25-ebpf-softirq-qualification-gating) |
| Connect ProcBuf payloads to hosted IPC | [09-25-procbuf-ipc-payload-adapter](https://github.com/ra1mj/ESOP/tree/c7d43e51ed0a5e12107bad8f55267087801568fe/.trellis/tasks/archive/2026-09/09-25-procbuf-ipc-payload-adapter) |
| Project runtime incidents into Protobuf and Zenoh | [09-25-runtime-incident-protobuf-zenoh](https://github.com/ra1mj/ESOP/tree/c7d43e51ed0a5e12107bad8f55267087801568fe/.trellis/tasks/archive/2026-09/09-25-runtime-incident-protobuf-zenoh) |
| Implement bounded Unix domain IPC transport | [09-25-unix-domain-ipc](https://github.com/ra1mj/ESOP/tree/c7d43e51ed0a5e12107bad8f55267087801568fe/.trellis/tasks/archive/2026-09/09-25-unix-domain-ipc) |
| Implement AL error acknowledgement and Device Emulation safety | [09-26-al-error-acknowledgement](https://github.com/ra1mj/ESOP/tree/c7d43e51ed0a5e12107bad8f55267087801568fe/.trellis/tasks/archive/2026-09/09-26-al-error-acknowledgement) |
| Publish CiA 402 feedback into ProcBuf state | [09-26-cia402-procbuf-state-feedback](https://github.com/ra1mj/ESOP/tree/c7d43e51ed0a5e12107bad8f55267087801568fe/.trellis/tasks/archive/2026-09/09-26-cia402-procbuf-state-feedback) |
| Write admitted commands into ProcBuf | [09-26-command-procbuf-target](https://github.com/ra1mj/ESOP/tree/c7d43e51ed0a5e12107bad8f55267087801568fe/.trellis/tasks/archive/2026-09/09-26-command-procbuf-target) |
| DC receive-time topology and propagation delay | [09-26-dc-receive-time-topology](https://github.com/ra1mj/ESOP/tree/c7d43e51ed0a5e12107bad8f55267087801568fe/.trellis/tasks/archive/2026-09/09-26-dc-receive-time-topology) |
| Execute ProcBuf targets through CiA 402 PDO | [09-26-procbuf-cia402-target-execution](https://github.com/ra1mj/ESOP/tree/c7d43e51ed0a5e12107bad8f55267087801568fe/.trellis/tasks/archive/2026-09/09-26-procbuf-cia402-target-execution) |
| Orchestrate product EtherCAT AL activation | [09-26-product-al-activation](https://github.com/ra1mj/ESOP/tree/c7d43e51ed0a5e12107bad8f55267087801568fe/.trellis/tasks/archive/2026-09/09-26-product-al-activation) |
| Generate deterministic product configuration artifacts | [09-26-product-config-generator](https://github.com/ra1mj/ESOP/tree/c7d43e51ed0a5e12107bad8f55267087801568fe/.trellis/tasks/archive/2026-09/09-26-product-config-generator) |
| Automate product PDO configuration batches | [09-26-product-pdo-batch](https://github.com/ra1mj/ESOP/tree/c7d43e51ed0a5e12107bad8f55267087801568fe/.trellis/tasks/archive/2026-09/09-26-product-pdo-batch) |
| Verify generated PDO configuration readback | [09-26-product-pdo-readback](https://github.com/ra1mj/ESOP/tree/c7d43e51ed0a5e12107bad8f55267087801568fe/.trellis/tasks/archive/2026-09/09-26-product-pdo-readback) |
| Integrate PDO configuration into production scheduler | [09-26-product-pdo-scheduler](https://github.com/ra1mj/ESOP/tree/c7d43e51ed0a5e12107bad8f55267087801568fe/.trellis/tasks/archive/2026-09/09-26-product-pdo-scheduler) |
| Attach generated product configuration to runtime | [09-26-product-runtime-attachment](https://github.com/ra1mj/ESOP/tree/c7d43e51ed0a5e12107bad8f55267087801568fe/.trellis/tasks/archive/2026-09/09-26-product-runtime-attachment) |
| Automatic slave-copy cycle-owner integration | [09-27-automatic-slave-copy-cycle-owner](https://github.com/ra1mj/ESOP/tree/c7d43e51ed0a5e12107bad8f55267087801568fe/.trellis/tasks/archive/2026-09/09-27-automatic-slave-copy-cycle-owner) |
| DC offset and propagation delay programming | [09-27-dc-offset-delay-programming](https://github.com/ra1mj/ESOP/tree/c7d43e51ed0a5e12107bad8f55267087801568fe/.trellis/tasks/archive/2026-09/09-27-dc-offset-delay-programming) |
| DC sync-window runtime monitoring | [09-27-dc-sync-window-runtime-monitoring](https://github.com/ra1mj/ESOP/tree/c7d43e51ed0a5e12107bad8f55267087801568fe/.trellis/tasks/archive/2026-09/09-27-dc-sync-window-runtime-monitoring) |
| Per-Domain cyclic WKC qualification | [09-27-domain-wkc-qualification](https://github.com/ra1mj/ESOP/tree/c7d43e51ed0a5e12107bad8f55267087801568fe/.trellis/tasks/archive/2026-09/09-27-domain-wkc-qualification) |
| ESC base and DC capability discovery | [09-27-esc-dc-capability-discovery](https://github.com/ra1mj/ESOP/tree/c7d43e51ed0a5e12107bad8f55267087801568fe/.trellis/tasks/archive/2026-09/09-27-esc-dc-capability-discovery) |
| ESC watchdog configuration and readback | [09-27-esc-watchdog-configuration](https://github.com/ra1mj/ESOP/tree/c7d43e51ed0a5e12107bad8f55267087801568fe/.trellis/tasks/archive/2026-09/09-27-esc-watchdog-configuration) |
| Implement ESM timeouts and OpOnly SyncManager safety | [09-27-esm-timeouts-op-only](https://github.com/ra1mj/ESOP/tree/c7d43e51ed0a5e12107bad8f55267087801568fe/.trellis/tasks/archive/2026-09/09-27-esm-timeouts-op-only) |
| EtherCAT asynchronous ESC register requests | [09-27-ethercat-async-register-requests](https://github.com/ra1mj/ESOP/tree/c7d43e51ed0a5e12107bad8f55267087801568fe/.trellis/tasks/archive/2026-09/09-27-ethercat-async-register-requests) |
| EtherCAT explicit recovery control plane | [09-27-ethercat-explicit-recovery](https://github.com/ra1mj/ESOP/tree/c7d43e51ed0a5e12107bad8f55267087801568fe/.trellis/tasks/archive/2026-09/09-27-ethercat-explicit-recovery) |
| EtherCAT explicit requesting ID | [09-27-ethercat-explicit-requesting-id](https://github.com/ra1mj/ESOP/tree/c7d43e51ed0a5e12107bad8f55267087801568fe/.trellis/tasks/archive/2026-09/09-27-ethercat-explicit-requesting-id) |
| EtherCAT explicit rescan | [09-27-ethercat-explicit-rescan](https://github.com/ra1mj/ESOP/tree/c7d43e51ed0a5e12107bad8f55267087801568fe/.trellis/tasks/archive/2026-09/09-27-ethercat-explicit-rescan) |
| EtherCAT runtime state requests | [09-27-ethercat-runtime-state-requests](https://github.com/ra1mj/ESOP/tree/c7d43e51ed0a5e12107bad8f55267087801568fe/.trellis/tasks/archive/2026-09/09-27-ethercat-runtime-state-requests) |
| EtherCAT single-slave reconfiguration | [09-27-ethercat-single-slave-reconfigure](https://github.com/ra1mj/ESOP/tree/c7d43e51ed0a5e12107bad8f55267087801568fe/.trellis/tasks/archive/2026-09/09-27-ethercat-single-slave-reconfigure) |
| FMMU-mapped mailbox status bit | [09-27-fmmu-mapped-mailbox-status-bit](https://github.com/ra1mj/ESOP/tree/c7d43e51ed0a5e12107bad8f55267087801568fe/.trellis/tasks/archive/2026-09/09-27-fmmu-mapped-mailbox-status-bit) |
| FMMU register descriptor discovery | [09-27-fmmu-register-discovery](https://github.com/ra1mj/ESOP/tree/c7d43e51ed0a5e12107bad8f55267087801568fe/.trellis/tasks/archive/2026-09/09-27-fmmu-register-discovery) |
| Generated mailbox status-bit discovery | [09-27-generated-mailbox-status-bit](https://github.com/ra1mj/ESOP/tree/c7d43e51ed0a5e12107bad8f55267087801568fe/.trellis/tasks/archive/2026-09/09-27-generated-mailbox-status-bit) |
| Product controlled Complete Access SDO | [09-27-product-complete-access-sdo](https://github.com/ra1mj/ESOP/tree/c7d43e51ed0a5e12107bad8f55267087801568fe/.trellis/tasks/archive/2026-09/09-27-product-complete-access-sdo) |
| Automate product mailbox configuration discovery | [09-27-product-mailbox-discovery](https://github.com/ra1mj/ESOP/tree/c7d43e51ed0a5e12107bad8f55267087801568fe/.trellis/tasks/archive/2026-09/09-27-product-mailbox-discovery) |
| Product-controlled SDO Information verification | [09-27-product-sdo-information-verification](https://github.com/ra1mj/ESOP/tree/c7d43e51ed0a5e12107bad8f55267087801568fe/.trellis/tasks/archive/2026-09/09-27-product-sdo-information-verification) |
| Product-owned slave-to-slave copy plans | [09-27-product-slave-copy](https://github.com/ra1mj/ESOP/tree/c7d43e51ed0a5e12107bad8f55267087801568fe/.trellis/tasks/archive/2026-09/09-27-product-slave-copy) |
| Bounded SII category stream discovery | [09-27-sii-category-stream-discovery](https://github.com/ra1mj/ESOP/tree/c7d43e51ed0a5e12107bad8f55267087801568fe/.trellis/tasks/archive/2026-09/09-27-sii-category-stream-discovery) |
| SII DC descriptor verification | [09-27-sii-dc-descriptor-verification](https://github.com/ra1mj/ESOP/tree/c7d43e51ed0a5e12107bad8f55267087801568fe/.trellis/tasks/archive/2026-09/09-27-sii-dc-descriptor-verification) |
| SII FMMU usage descriptor verification | [09-27-sii-fmmu-usage-verification](https://github.com/ra1mj/ESOP/tree/c7d43e51ed0a5e12107bad8f55267087801568fe/.trellis/tasks/archive/2026-09/09-27-sii-fmmu-usage-verification) |
| Integrate live SII mailbox verification into Startup | [09-27-startup-sii-mailbox-verification](https://github.com/ra1mj/ESOP/tree/c7d43e51ed0a5e12107bad8f55267087801568fe/.trellis/tasks/archive/2026-09/09-27-startup-sii-mailbox-verification) |
| Startup-owned SII stream verification | [09-27-startup-sii-stream-verification](https://github.com/ra1mj/ESOP/tree/c7d43e51ed0a5e12107bad8f55267087801568fe/.trellis/tasks/archive/2026-09/09-27-startup-sii-stream-verification) |
| SyncManager register bank discovery | [09-27-sync-manager-register-discovery](https://github.com/ra1mj/ESOP/tree/c7d43e51ed0a5e12107bad8f55267087801568fe/.trellis/tasks/archive/2026-09/09-27-sync-manager-register-discovery) |
| Topology-wide DC SYNC programming | [09-27-topology-wide-dc-sync-programming](https://github.com/ra1mj/ESOP/tree/c7d43e51ed0a5e12107bad8f55267087801568fe/.trellis/tasks/archive/2026-09/09-27-topology-wide-dc-sync-programming) |
| EtherCAT recovery integration and qualification | [09-28-ethercat-recovery-integration](https://github.com/ra1mj/ESOP/tree/c7d43e51ed0a5e12107bad8f55267087801568fe/.trellis/tasks/archive/2026-09/09-28-ethercat-recovery-integration) |
| Runtime diagnostics CLI and eBPF status | [09-28-runtime-diagnostics-cli-ebpf](https://github.com/ra1mj/ESOP/tree/c7d43e51ed0a5e12107bad8f55267087801568fe/.trellis/tasks/archive/2026-09/09-28-runtime-diagnostics-cli-ebpf) |

## Runtime diagnostic child

The completed `09-28-runtime-diagnostics-cli-ebpf` child delivered the
typed read-only namespace, ProcBuf projections and the original library
router tests. Its original acceptance did not execute the `esop` process.
The October audit adds binary startup, every view/watch form, failure
exits and incident-only update coverage. The current
[CLI contract](esop-runtime-diagnostics-cli.md) describes the corrected behavior.
