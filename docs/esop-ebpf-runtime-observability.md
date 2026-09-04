# ESOP eBPF 运行时观测与问题归因设计

- 文档版本：1.0
- 日期：2026-09-03
- 状态：设计基线；HostObservation、固定证据 ABI、有界 RuntimeIncident 相关器、同类事件窗口聚合、RuntimeAgent 门面、能力预检结果模型、Rust/Aya CO-RE loader、tracepoint attach 和 ringbuf 解码桥已实现；目标 BPF ELF 构建与生产 hook 资格仍需在目标 Linux 环境完成
- 上游需求：[ESOP 软件产品需求文档](esop-software-prd.md) FR-047 至 FR-052、NFR-018

## 1. 设计结论

ESOP 增加一个只运行在 Linux 监督域的 `esop-ebpf-agent`。它在不修改被观测业务逻辑的情况下，从 Linux 内核和 ESOP/ROS 2/Zenoh 用户态采集运行时证据，聚合成 `RuntimeIncident`，并与 ESOP 的 cycle、WKC、DC、ProcBuf 和 MLG 状态关联。

eBPF 是观测层，不是 EtherCAT 实时核心，也不是功能安全通道：

1. STM32/HPMicro 裸机或 RTOS 节点不运行 eBPF；它们继续使用 ESOP 固定事件、计数器、ProcBuf 和端口诊断。
2. eBPF agent 不拥有 CiA 402 controlword、motion permit 或 MLG 状态的写权限。
3. eBPF agent 发现主机问题后，可以生成 `HOST_OBSERVATION` 证据、撤销监督心跳或请求普通控制域停止；最终运动许可仍由 MLG 在实时域裁决。
4. eBPF agent 不可用时，系统必须报告“观测降级”，但不能因此让 RT 核心等待它；只有产品明确把主机观测列为必需门槛时，MLG 才可根据失去监督租约阻止运动。

## 2. 为什么采用 eBPF

运行时问题经常跨越多个层次：ROS 2 controller 的 `write()` 可能正常返回，但 Linux 调度延迟、网卡 IRQ 拥塞、页错误、CPU 限频或 Zenoh gateway 停顿已经使命令错过 deadline。普通应用日志通常只能说明“结果异常”，无法说明异常发生在进程、线程、CPU、IRQ、网络或内存的哪一层。

eBPF 适合做这类关联观测，因为它可以挂接 Linux tracepoint、kprobe、fentry/fexit、uprobe 等观测点，并在内核侧用 map、计数器和直方图聚合数据；用户态再通过固定事件通道读取结果。eBPF 程序必须通过内核 verifier，不能把任意不受控代码直接装入内核。

产品采用 libbpf + CO-RE 加载方式。CO-RE 需要运行内核提供 BTF；BPF ring buffer 作为默认事件通道，使用固定容量、非阻塞的 reserve/commit 或 output 语义。若目标 Linux 内核不具备所需 BTF、ring buffer 或 attach 能力，agent 应进入 `DEGRADED` 并使用有限的用户态/ProcBuf 诊断，不得在运行期编译或阻塞等待内核能力。

## 3. 运行位置与数据流

```text
Linux kernel
  tracepoints / fentry-fexit / kprobe / uprobe / perf counters
        -> BPF maps, histograms, bounded ringbuf
        -> esop-ebpf-agent
             -> incident correlator
             -> RuntimeIncident + observation heartbeat
             -> Protobuf / Zenoh / local recorder
             -> supervisor health lease -> MLG input

ESOP RT node
  ProcBuf lifecycle + WKC/DC/deadline + fixed event ring
        -> IPC -> Linux correlator
```

两条证据链保持独立：RT 域是运动控制事实来源；eBPF 是 Linux 环境的解释与归因来源。相关器可以合并“同一个周期窗口内的事件”，但不能以缺少 eBPF 事件证明“系统没有问题”。

当前代码已在 `crates/esop-lifecycle-guard/` 落地固定大小的 `HostObservation`、`agent_epoch`/`heartbeat_seq` 防重放、单调时间年龄校验和 `HostObservation` 生命周期门槛；`crates/esop-ebpf-agent/` 已落地固定证据 ABI、cycle/WKC/DC 风险关联、有界 incident 环、同一代码/组件/时间窗口内的证据聚合、incident 有界消费、`RuntimeAgent` 健康租约门面和 BTF/ringbuf/verifier/permission/attach 能力预检结果模型。`crates/esop-ebpf-runtime/` 现在提供实际的 Rust/Aya BPF ELF loader、逐点 tracepoint attach、固定 96 字节事件解码、kernel context map 更新、per-CPU 丢失计数读取和 `RuntimeAgent` 桥接；`bpf/` 提供首版内核程序源与构建入口。目标 Linux 环境仍需使用 clang 生成 BPF ELF，并完成真实权限、verifier、ringbuf 和目标 hook 资格测试。

## 4. 观测域与 attach 点

### 4.1 内核观测点

| 域 | 观测点/机制 | 能回答的问题 | 首版输出 |
| --- | --- | --- | --- |
| 调度 | `sched_switch`、`sched_wakeup`、`sched_process_exec/exit` | RT 线程何时被唤醒、实际运行多久、被谁抢占、是否迁移、进程是否退出。 | run-queue latency、off-CPU time、migration、thread exit。 |
| IRQ/softirq | IRQ 与 softirq entry/exit、CPU 时间统计 | 哪个 IRQ/softirq 占满 CPU，Ethernet IRQ 是否延迟服务。 | handler duration、storm count、CPU overlap。 |
| 网络 | `net_dev_queue`、`net_dev_xmit`、`netif_receive_skb`、`napi_poll`、`kfree_skb` 等适用点 | EtherCAT raw port 的帧是否进入/离开队列，是否被丢弃或 NAPI/IRQ 延迟。 | interface、queue、drop reason、receive/transmit latency。 |
| 内存 | user page fault、OOM、进程 mmap/munmap 等低频点 | 周期或 gateway 是否发生页错误、内存压力或 OOM。 | fault count、major/minor、OOM、address-space change。 |
| 进程 | exec/exit、signal、cgroup、CPU throttling/pressure 可用点 | gateway、ROS 2、Zenoh、recorder 是否重启、被杀或受资源组限制。 | pid/tid、exit code/signal、cgroup、throttle window。 |
| 性能计数 | kernel perf event 或平台可用 PMU | CPU cycles、instructions、cache miss 等是否突然恶化。 | 只做采样/窗口统计，不在每周期输出原始样本。 |

实际 attach 点由内核版本、BTF、发行版和可用 tracepoint 决定。agent 启动时必须发布 attach 成功/失败清单，不允许假定所有 Linux 内核都有同一组内核函数或字段。

### 4.2 用户态观测点

首版优先使用稳定的 ESOP 用户态符号或显式 trace hook，通过 uprobe/uretprobe 观测：

1. `esop_ros2_control` 的 `read()`、`write()` 和 controller update 边界。
2. `esop_zenoh_gateway` 的 IPC read/write、序列化、publish/query、permit accept/reject 和 reconnect。
3. Linux RT port 的 cycle begin/end、RX drain、commit、prepare、send 和 error return。
4. recorder、配置工具和维护进程的启动、退出、阻塞和异常返回。

这些 hook 必须携带固定的 `boot_id`、`cycle_seq` 或 `request_id`，不得在 uprobe 中解析动态字符串。对外发布的用户态 hook 需保持 ABI 版本；符号缺失时 agent 标记 `USER_PROBE_UNAVAILABLE`，不影响 RT 核心。

## 5. 事件与关联模型

### 5.1 RuntimeIncident

每个事件使用固定上限的结构化记录：

```text
RuntimeIncident
  incident_id
  boot_id / host_id / agent_epoch
  severity / code / source_domain
  first_seen_ns / last_seen_ns / evidence_window_ns
  esop_cycle_first / esop_cycle_last
  lifecycle_transition_seq
  pid / tid / cgroup / cpu / irq / netdev
  observed_value / threshold / count / duration_ns
  related_wkc / dc_offset / command_age / input_age
  lost_events / attach_mask / config_hash
  recommended_action
```

`RuntimeIncident` 是观测事实和推断结果的容器。`source_domain` 区分 `KERNEL_SCHED`、`KERNEL_IRQ`、`KERNEL_NET`、`KERNEL_MM`、`USER_ESOP`、`USER_ROS`、`USER_ZENOH` 和 `CORRELATOR`。推断结果必须列出支持它的原始事件 ID 或窗口，不能把推断写成未经解释的根因。

### 5.2 关联键

相关器按以下顺序关联事件：

1. `boot_id`：上位机/RT 节点启动实例变化时，旧事件不得与新事件混合。
2. `cycle_seq`：ESOP Linux 端在 cycle begin/end 处发出固定序号；RT ProcBuf 也携带同一逻辑序号或映射关系。
3. `transition_seq`：MLG 生命周期状态变化的因果序号。
4. `pid/tid/cpu/netdev/irq`：将主机资源事件映射到具体组件。
5. `monotonic time window`：关联前后固定窗口内的调度、IRQ、网络和内存事件。

时间关联必须记录校准状态和误差上限。不能用 wall clock、ROS time 或日志打印时间代替实时单调时间。

## 6. 问题检测规则

### 6.1 首版问题分类

| 代码 | 触发证据 | 结论 | MLG 关系 |
| --- | --- | --- | --- |
| `HOST_SCHEDULER_STALL` | RT/gateway 线程唤醒到运行的延迟超过阈值，且与 cycle deadline miss 同窗。 | 主机调度导致软件周期风险。 | 可使 supervisor lease 失效；不直接写 controlword。 |
| `HOST_IRQ_STORM` | 单 IRQ/softirq 在窗口内占用超预算 CPU，伴随 RT 线程 off-CPU。 | 中断或软中断干扰实时线程。 | 按产品策略触发普通 controlled stop。 |
| `HOST_NIC_DROP` | 网卡/协议栈丢帧、队列溢出或 receive/transmit gap 与 WKC/timeout 同窗。 | Linux 网络路径有丢包或拥塞。 | 提供 `HOST_OBSERVATION`，不能替代 EtherCAT WKC。 |
| `HOST_PAGE_FAULT` | ESOP/ROS/Zenoh 关键线程在 cycle 窗口发生页错误。 | 周期可能被内存管理事件打断。 | 性能资格失败或按策略撤销 host permit。 |
| `HOST_CPU_THROTTLE` | cgroup/CPU pressure/频率窗口异常与 gateway stall 同窗。 | 监督域资源受到限制。 | supervisor lease 降级。 |
| `USER_COMPONENT_EXIT` | gateway、ROS 2 controller、recorder 或 agent 退出/收到信号。 | 用户态组件生命周期异常。 | MLG 只根据固定 supervisor lease/command age 判定。 |
| `OBSERVABILITY_DEGRADED` | BTF/attach/permission/ringbuf/agent health 失败。 | 观测证据不完整。 | 不能自动声称健康；是否禁止运动由产品 policy 决定。 |

### 6.2 检测规则原则

1. 每条规则包含 `enter_threshold`、`exit_threshold`、`window_ns`、`min_count`、`max_age_ns` 和严重级别。
2. 连续周期、时间窗口和滞回逻辑由用户态相关器执行；内核程序只做轻量采集和聚合。
3. 单个 eBPF 事件不直接判定根因；至少需要 ESOP/MLG 状态或第二类主机证据进行关联，除非是明确的进程退出、OOM 等硬事实。
4. 所有规则都保留 `observed`、`threshold`、`evidence_ids` 和 `confidence`，并区分事实、相关性和推断。
5. `RuntimeIncident` 产生后不能覆盖 RT 原始事件；事件环满、ringbuf 满或 agent 重启都必须记录丢失计数。

## 7. 与 MLG 的安全连接

### 7.1 不允许的连接

eBPF 程序和 agent 不得：

1. 直接写 EtherCAT 帧、PDO、CiA 402 controlword、motion permit 或 MLG state。
2. 调用 ROS 2 executor、Zenoh router、文件系统或网络服务来完成内核事件处理。
3. 以“未观察到异常”清除 `FAULT_LATCHED`、恢复 permit 或允许 `MOTION_ACTIVE`。
4. 在 RT 线程中同步等待 ringbuf consumer、agent、内核事件或用户态分析结果。

### 7.2 允许的连接

agent 通过固定 IPC 向监督域发送：

```text
host_observation_snapshot
  agent_epoch
  observation_state: HEALTHY / DEGRADED / FAILED
  attach_mask
  last_event_ns
  lost_event_count
  incident_count
  host_gate_bits
  supervisor_heartbeat_seq
```

监督域只将上述数据转换为带 TTL 的 `host_health_lease`。实时 MLG 以自己的策略验证 lease 的 boot ID、epoch、序号和时效；lease 过期时，MLG 可以撤销普通控制 permit，但仍按既定停止策略执行，不能由 agent 越权控制驱动。

### 7.3 eBPF agent 失效策略

| 失效 | agent 状态 | 默认 MLG 行为 |
| --- | --- | --- |
| 单个 attach 点不可用 | `DEGRADED` | 继续使用其他证据；发布缺失能力。 |
| ringbuf 满/事件丢失 | `DEGRADED` | 计数、降低采样或只保留 incident；不阻塞。 |
| agent 用户进程重启 | `RESTARTING` | 旧 epoch 失效；监督 lease 进入宽限期。 |
| BTF/权限/加载失败 | `FAILED` | 仅按产品 policy 决定是否禁止 host 侧运动；RT 仍不等待。 |
| 关键 host gate 明确失效 | `FAILED` | 监督域撤销 lease，MLG 执行 configured stop。 |

## 8. 性能与资源约束

1. eBPF 程序不进行无限循环、动态字符串构造或阻塞等待；每次触发只读取固定大小上下文。
2. 高频点使用 per-CPU counter/histogram 或聚合 map，禁止为每次调度切换发送完整 Protobuf。
3. ringbuf 使用固定的 2 的幂容量；满时 reserve/output 失败必须非阻塞返回并增加 `lost_event_count`。
4. agent 用户态消费采用 epoll/批量消费和低优先级线程；事件分析和 Protobuf/Zenoh 发布在观测线程执行。
5. 观测配置分为 `baseline`、`incident` 和 `forensics`：baseline 低开销聚合，incident 窗口短时提高细节，forensics 仅维护模式使用。
6. 每次资格测试报告 eBPF 程序运行次数/时间、agent CPU/RAM、ringbuf 水位、丢失事件、attach 能力和对 gateway/host RT 延迟的影响。
7. eBPF 观测开销不计入 STM32/HPMicro EtherCAT 周期，但必须计入 Linux 端 Q3/`split-linux-rt` 性能资格；不得用打开 eBPF 的结果替代 MCU 资格结论。

## 9. 权限、版本与部署

agent 启动前执行 capability preflight：内核版本与 BPF 功能、`/sys/kernel/btf/vmlinux`、libbpf/CO-RE、所需 map/attach 类型、运行身份和资源限制。预检结果写入 capability manifest。

BPF 对象、用户态 loader、schema 和规则版本必须绑定：

1. `ebpf_bundle_version` 与 ESOP release 版本关联。
2. 每个 BPF 程序记录 program name、attach target、load result、verifier error 摘要和 kernel BTF identity。
3. 规则变化产生 `observation_policy_hash`，不能静默替换运行中的阈值。
4. agent 只能加载来自受信任发布物的 BPF object；生产环境默认只读挂载、最小权限和独立 systemd/cgroup 资源限制。
5. `single-host-dev` 可允许更宽松的 attach 和 debug 输出；量产环境禁止 `bpf_printk` 作为事件通道。

## 10. 验收与测试矩阵

| ID | 类型 | 验收要求 |
| --- | --- | --- |
| EBPF-001 | 能力 | 在支持/不支持 BTF、ringbuf、attach 点和权限的内核上，agent 给出可解释的 capability manifest。 |
| EBPF-002 | 加载 | BPF verifier 拒绝、CO-RE relocation 失败、attach 失败和 agent 卸载均不会破坏 ESOP/ROS/Zenoh 主流程。 |
| EBPF-003 | 调度 | 人为注入线程延迟、CPU 迁移、IRQ/softirq 压力，能生成 `HOST_SCHEDULER_STALL` 或 `HOST_IRQ_STORM` 证据。 |
| EBPF-004 | 网络 | 注入网卡队列、协议栈丢包和 raw port 延迟，能关联 `HOST_NIC_DROP` 与 EtherCAT WKC/timeout 窗口。 |
| EBPF-005 | 内存 | 注入页错误、内存压力和进程 OOM/退出，能生成 `HOST_PAGE_FAULT`/`USER_COMPONENT_EXIT`。 |
| EBPF-006 | 用户态 | gateway、`ros2_control`、recorder 重启或函数超时能按 PID/TID、cycle_seq 和 request_id 归因。 |
| EBPF-007 | 完整性 | agent 无法写 MLG、controlword、permit；伪造/过期 host observation 不可清除 fault latch。 |
| EBPF-008 | 降级 | ringbuf 满、事件丢失、agent 重启、BTF 缺失、权限不足时，丢失计数和 `OBSERVABILITY_DEGRADED` 可见。 |
| EBPF-009 | 相关性 | 生成包含 RT 事件、ProcBuf 状态、MLG transition 和 eBPF evidence ID 的 incident timeline。 |
| EBPF-010 | 开销 | baseline/incident/forensics 三档测得 CPU、内存、ringbuf、事件丢失和 host RT 影响；不改变 MCU 资格结论。 |
| EBPF-011 | 长测 | 至少 30 分钟 Q1/Q2 Linux 监督域压力测试，无 agent 内存增长、无无限 map 增长、无周期阻塞。 |
| EBPF-012 | 安全边界 | 产品测试报告明确 eBPF 不是安全通道；STO/FSoE/安全 PLC 仍独立验证。 |

## 11. 运行时输出示例

一次周期异常的可解释链路应类似：

```text
cycle 184220: deadline_miss
  -> MLG transition_seq 882: HOST_OBSERVATION degraded
  -> USER_ESOP: gateway write() delayed 1.8 ms
  -> KERNEL_SCHED: gateway TID runqueue latency 1.2 ms on CPU 3
  -> KERNEL_IRQ: eth IRQ/softirq consumed 0.9 ms in same window
  -> KERNEL_NET: RX queue drop count +4 on eth0
  -> EtherCAT: Domain actual WKC < expected, input age +1
  -> action: revoke host lease, configured stop=QUICK_STOP, await recovery
```

该链路中的“事实”来自各自的原始事件；`CORRELATOR` 只说明它们在同一个 cycle/time window 内相关，不把相关性伪装成经过形式证明的单一根因。

## 12. 当前未决项

1. 量产 Linux 内核最低版本、是否强制 CONFIG_DEBUG_INFO_BTF、目标发行版及 libbpf 版本。
2. Linux raw port、`esop_ros2_control`、Zenoh gateway 的稳定用户态 hook 名称与 ABI。
3. 关键 host gate 是否作为 split-linux-rt 的运动前提，以及其宽限期和停止策略。
4. 目标网卡的可观测 tracepoint、驱动特定 attach 点、RX/TX queue 映射与丢包口径。
5. baseline/incident/forensics 的采样率、数据留存时长、隐私字段和远程上传策略。
6. 生产环境 eBPF 加载权限、签名/完整性保护、systemd/cgroup 隔离和升级回滚流程。

## 13. 参考依据

- Linux Kernel Documentation：BPF verifier、BPF ring buffer、libbpf、BTF、program types。
- libbpf 官方文档与 `libbpf-bootstrap`：CO-RE 应用生命周期、uprobe 示例、ring buffer 与旧内核兼容思路。
- 本设计不把 eBPF 的存在、加载成功或观测结果当作功能安全认证证据；安全结论仍由独立安全需求、风险评估和验证流程给出。
