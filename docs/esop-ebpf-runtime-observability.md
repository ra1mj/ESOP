# ESOP eBPF 运行时观测与问题归因设计

- 文档版本：1.2
- 日期：2026-09-25
- 状态：设计基线；HostObservation、固定证据 ABI、有界 RuntimeIncident 相关器、同类事件窗口聚合、RuntimeAgent 门面、能力预检结果模型、Rust/Aya CO-RE loader、tracepoint attach、ringbuf 解码桥、有界硬 IRQ/softirq 时长证据、按 EtherType/ifindex 聚合的 `kfree_skb` 丢包证据、有界 cpufreq policy 限频 episode、Zenoh gateway stall 和 Linux raw-port syscall stall 证据已实现；托管 Linux 已覆盖 gateway/raw-port 共享 marker、进程 leader 退出、独立 cgroup v2 单进程 OOM victim、精确 TID 调度迁移、受控 wake-to-switch runqueue、受控 loopback `NET_RX` softirq 和受控 x86_64 匿名页首次写入 page-fault 链的 verifier/load/真实 attach/ringbuf/相关器资格，生产目标内核、真实 transport/NIC/自然负载根因、全局 OOM/受害 TGID 与 cgroup 归因、硬 IRQ 与持续压力/限频注入、页错误内存压力/major-minor 归因、开销和其余 hook 资格仍需单独完成
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

当前代码已在 `crates/esop-lifecycle-guard/` 落地固定大小的 `HostObservation`、`agent_epoch`/`heartbeat_seq` 防重放、单调时间年龄校验和 `HostObservation` 生命周期门槛；`crates/esop-ebpf-agent/` 已落地固定证据 ABI、cycle/WKC/DC 风险关联、有界 incident 环、同一代码/组件/时间窗口内的证据聚合、incident 有界消费、`RuntimeAgent` 健康租约门面和 BTF/ringbuf/verifier/permission/attach 能力预检结果模型。`crates/esop-ebpf-runtime/` 现在提供实际的 Rust/Aya BPF ELF loader、逐点 tracepoint attach、固定 96 字节事件解码、kernel context map 更新、per-CPU 统计读取、调度 TID/迁移计数窗口策略原子更新、硬 IRQ/softirq entry/exit attach、IRQ/softirq CPU/vector 过滤、EtherCAT EtherType/可选 ifindex 丢包策略更新、页错误计数窗口策略更新、cpufreq policy 下限/CPU 策略原子更新、Zenoh gateway 与 Linux raw-port syscall 成对 uprobe attach 和 `RuntimeAgent` 桥接；`bpf/` 提供固定 1024 项的调度 TID 迁移窗口 map、固定容量中断起始时间 map、固定 256 项的 CPU/ifindex 丢包窗口 map、固定 256 项的 CPU/进程页错误窗口 map、固定 256 项的 cpufreq policy episode map、固定 1024 项 gateway request 与 raw-port per-thread 操作 map、主线程退出过滤、OOM victim PID 归因、阈值事件和统计计数。`crates/esop-procbuf/tests/cross_layer.rs` 已验证健康心跳可通过 MLG 观测门槛，能力退化心跳会触发配置的 Quick Stop。专用托管 Linux CI 已验证 gateway、raw-port、leader 退出、独立 cgroup v2 单进程 OOM victim、两 CPU 精确 TID 调度迁移、受控 FIFO 竞争下精确 TID 唤醒到切换时长、CPU/vector 过滤的 loopback `NET_RX` softirq 时长，以及预热独立子进程的 16 页匿名内存首次写入 page-fault 计数阈值链的 CO-RE verifier/load、真实 tracepoint/uprobe attach、ringbuf、统计、相关器和 observer health 共享链；生产目标内核与真实 Zenoh transport、AF_PACKET/NIC、自然负载 runqueue 根因、硬 IRQ、真实 NIC/持续 softirq、丢包/页错误内存压力与 major-minor 归因、全局 OOM/受害 TGID/namespace/cgroup 归因、限频压力、迁移原因/亲和性/cache 影响及开销资格仍需单独完成。

## 4. 观测域与 attach 点

### 4.1 内核观测点

| 域 | 观测点/机制 | 能回答的问题 | 首版输出 |
| --- | --- | --- | --- |
| 调度 | `sched_switch`、`sched_wakeup`、`sched_migrate_task`、`sched_process_exec/exit` | RT 线程何时被唤醒、实际运行多久、被谁抢占、是否迁移、进程是否退出。 | run-queue latency、migration count/source/destination、thread exit。 |
| IRQ/softirq | IRQ 与 softirq entry/exit、CPU 时间统计 | 哪个 IRQ/softirq 占满 CPU，Ethernet IRQ 是否延迟服务。 | handler duration、storm count、CPU overlap。 |
| 网络 | `net_dev_queue`、`net_dev_xmit`、`netif_receive_skb`、`napi_poll`、`kfree_skb` 等适用点 | EtherCAT raw port 的帧是否进入/离开队列，是否被丢弃或 NAPI/IRQ 延迟。 | interface、queue、drop reason、receive/transmit latency。 |
| 内存 | user page fault、OOM、进程 mmap/munmap 等低频点 | 周期或 gateway 是否发生页错误、内存压力或 OOM。 | fault-window count、架构错误码、OOM、address-space change；major/minor 结果需其他 hook。 |
| 进程 | exec/exit、signal、cgroup、CPU throttling/pressure 可用点 | gateway、ROS 2、Zenoh、recorder 是否重启、被杀或受资源组限制。 | pid/tid、exit code/signal、cgroup、throttle window。 |
| 性能计数 | kernel perf event 或平台可用 PMU | CPU cycles、instructions、cache miss 等是否突然恶化。 | 只做采样/窗口统计，不在每周期输出原始样本。 |

实际 attach 点由内核版本、BTF、发行版和可用 tracepoint 决定。agent 启动时必须发布 attach 成功/失败清单，不允许假定所有 Linux 内核都有同一组内核函数或字段。

调度迁移首版使用 `sched:sched_migrate_task` typed context，读取 scheduler
entity PID/TID、priority、origin CPU 和 destination CPU。产品可配置独立的
scheduler TID；零值回退到现有 `tracked_pid`，两者均为零时保留全任务开发
模式。每个 TID 在固定 1024 项 LRU map 中维护单调窗口、饱和迁移计数和策略
epoch，只在窗口第一次达到计数阈值时发送一条 96 字节 `CpuMigration` 证据。
证据 PID 为零、TID 为 tracepoint PID、`cpu` 为 destination CPU、kind-specific
`irq` 槽为 origin CPU、`detail` 为饱和 priority。该证据必须与 deadline、WKC
或 DC 风险周期同窗，才以低于实测 runqueue latency 的置信度合并为
`HOST_SCHEDULER_STALL`。单次迁移和健康周期不构成事故；事件也不证明迁移
原因、亲和性错误、cache/NUMA 影响或实际停顿时长。`sched_switch` 的
runqueue-latency 证据同样显式记录被调度的 `next_pid` 为 TID，不再使用 hook
执行上下文的 current task 身份。

专用 `make test-ebpf-scheduler-migration-runtime` 在至少允许两个 CPU 的特权
托管 Linux 上只要求 `sched_migrate_task`，把持续可运行 worker 固定到 CPU A 后
启用精确 TID 跟踪，再以 singleton affinity 强制 A→B→A。第一次迁移必须仅增加
`scheduler_migrations`，第二次才允许唯一阈值事件、Warning/confidence-60
`HostSchedulerStall`/`DegradeHostObservation` 和 Degraded heartbeat；完整字段原子
写入 `build/ebpf_scheduler_migration_qualification.json` 并做封闭 schema 校验。
该资格证明 hosted-kernel migration-to-incident 链，不把窗口跨度解释为停顿时长，
也不证明迁移原因、亲和性策略错误、cache/NUMA 影响或生产实时性能。

专用 `make test-ebpf-scheduler-runqueue-runtime` 在至少允许两个 CPU 且可进入
`SCHED_FIFO` 的特权托管 Linux 上只要求 `sched_wakeup` 与 `sched_switch`。普通
target 固定在 CPU A 并进入私有 futex 睡眠，控制线程固定在 CPU B；CPU A 的有界
FIFO blocker 激活后，控制线程唤醒唯一 waiter 并要求 target 在 25 ms hold 内保持
未完成。释放 blocker 后，`sched_switch.next_pid` 必须匹配精确 target TID，并生成
唯一 Error `SchedulerRunqueueLatency`、confidence-70 `HostSchedulerStall`/
`ControlledStop` 和 Degraded heartbeat。完整前提、时长、统计、ringbuf、cycle、
incident 与健康字段原子写入
`build/ebpf_scheduler_runqueue_qualification.json` 并做封闭 schema 校验。该资格只
证明 hosted-kernel controlled wake-to-switch 共享链，不推断自然负载根因、普通
调度行为、产品优先级/CPU 隔离策略、生产内核、开销或 WCET。

当前首版以 `irq_handler_entry`/`irq_handler_exit` 和
`softirq_entry`/`softirq_exit` 计算单次 handler 时长。硬 IRQ 与 softirq
分别使用 `IrqCpuTime` 和 `SoftirqCpuTime` 证据 discriminant，保持既有
96 字节事件布局；`irq` 的 `u16` 范围外编号饱和而不回绕。起始时间 map
以 CPU/vector 为键并设置固定容量。entry/exit
attach 必须成对启用，阈值事件仍需与 transport-risk cycle 同窗才升级为
`HOST_IRQ_STORM`。loader 对每组 pair 执行成组挂载；第二个成员失败时回滚
第一个 link，capability mask 也不会发布半组能力。

专用 `make test-ebpf-softirq-runtime` 在特权托管 Linux 上从实际 allowed affinity
选择并固定一个安静 CPU，只要求 softirq pair，并在 entry 写入起始 map 前精确过滤
该 CPU 与 `NET_RX` vector 3。夹具通过一次 `UDP_SEGMENT=1200` 的 64,800 字节
loopback 发送产生 54 个 datagram；一次独立校准运行测得完整 vector action 时长，
正式运行使用 `max(1, calibration_duration_ns / 8)` 阈值并要求恰好一次目标 CPU
`NET_RX`、一条 `KernelIrq/SoftirqCpuTime/Error`、一个 confidence-70
`HostIrqStorm`/`ControlledStop` incident、零 loss 和 Healthy-to-Degraded heartbeat。
CPU/vector、socket、校准、统计、cycle、incident 和健康字段原子写入
`build/ebpf_softirq_qualification.json` 并由封闭 schema 校验。PID/TID 只表示 inline
softirq 返回时的 task context，不表示 softirq 所有权或根因。该资格不覆盖硬 IRQ、
真实 NIC/driver/NAPI、产品中断预算、持续压力、生产内核、开销或 WCET。

网络首版使用 `skb:kfree_skb` 的 typed tracepoint context，只接受配置的
host-order EtherType（默认 EtherCAT `0x88A4`），并可选精确匹配 ifindex。
程序通过 CO-RE 读取 `skb->dev->ifindex`，无法取得时回退 `skb_iif`；仍无法
解析的协议匹配事件只增加 `network_unattributed`，不生成事故证据。可归因
事件按 `{CPU, ifindex}` 写入固定 256 项 LRU map，在固定窗口第一次达到计数
阈值时输出一条 96 字节证据，携带 ifindex、窗口计数和饱和为一字节的内核
drop reason。由于软中断上下文中的 current task 不代表数据包所有者，网络
证据的 PID/TID 固定为零，也不使用 `tracked_pid` 过滤。

页错误首版使用 `exceptions:page_fault_user` 的 typed tracepoint context，并
按 `tracked_pid` 过滤。每个 `{CPU, 进程}` 在固定 256 项 LRU map 中维护窗口
起点和饱和计数；窗口第一次达到配置阈值时输出一条 96 字节证据，携带触发
PID/TID、CPU、窗口计数/跨度以及饱和为一字节的架构错误码。该 tracepoint
没有页错误处理完成点或 major/minor 结果，因此当前实现不把窗口跨度解释为
处理时延，也不从错误码推断 major/minor。页错误证据还必须与 deadline、WKC
或 DC 风险周期同窗，才可升级为 `HOST_PAGE_FAULT`。

专用 `make test-ebpf-page-fault-runtime` 在特权托管 x86_64 Linux 上只挂载
`exceptions:page_fault_user`。夹具在加载 BPF 前启动并固定独立子进程，预热代码、
栈和控制管道，准备 16 个带 guard page 且禁用 THP 的匿名映射；加载完成并把
`tracked_pid` 指向子进程 TGID 后，才放行每个未驻留页的一次首次写入。子进程在
BPF 拆卸前保持存活，避免退出清理页错误污染窗口。资格同时要求 `ru_minflt`
增量、BPF `page_faults` 和证据 count 精确为 16，只生成一个
`KernelMemory/PageFault`、Warning/confidence-65 `HostPageFault`/
`DegradeHostObservation` incident、零 loss 和 fault `0x45422001` 的 Degraded
heartbeat；完整报告写入 `build/ebpf_page_fault_qualification.json` 并做闭合 schema
校验。x86_64 错误码 detail 只作为受控 user/write/not-present 触发条件的原始佐证，
不扩展为 fault address/IP、major/minor、handler duration、内存压力或根因判断。

进程生命周期首版把 `sched:sched_process_exit` 视为线程级事件。程序先按当前
TGID 应用 `tracked_pid` 过滤，仅当退出 TID 等于 TGID 时生成一次
`ProcessExit` 硬事实；受跟踪进程的普通工作线程退出只增加
`thread_exits_ignored`，不会触发 `USER_COMPONENT_EXIT`。这条信号表示主线程
退出，不声称线程组中所有任务已经消失，也不包含 exit code 或 signal。
专用 `make test-ebpf-process-exit-runtime` 会在特权托管 Linux 上仅挂载该
tracepoint，把跟踪 PID 切换到同步子进程后先创建并 join 一个 worker，要求它
只增加 ignored 统计且不改变 Healthy lease；随后放行 leader 正常退出，要求真实
CO-RE verifier/load、ringbuf、统计、`UserComponentExit`/`LatchFault` 和 Failed
heartbeat 全链自洽，并原子生成
`build/ebpf_process_exit_qualification.json`。该资格仍不提供退出原因或线程组
完全死亡证明。

OOM 首版从 `oom:mark_victim` typed context 读取内核选中的 victim PID，而不
使用触发 OOM killer 的 current task。固定证据的 PID/TID 都写入该 victim
PID，因为该 tracepoint 不提供独立 TGID。`tracked_pid == 0` 时保留所有正
victim PID；非零时只接受精确 PID 匹配，因此内核若选中同一进程的其他线程，
当前规则可能漏报。解决线程到进程成员关系需要额外有界身份来源或不同 hook。

CPU 限频首版使用 `power:cpu_frequency_limits` 的 typed context，读取 cpufreq
policy 的代表 `cpu_id` 和 `max_freq`。产品必须提供非零的资格频率下限（kHz），
并可精确选择一个 policy CPU；默认全 policy 模式使用显式 sentinel，不占用
CPU 0。每个 policy CPU 在固定 256 项 LRU map 中维护 episode 状态，首次
`max_freq` 低于下限时发送一条 96 字节证据，同一 episode 后续低频更新只计数，
直到 `max_freq` 恢复到下限或以上才重新武装。运行期策略更新递增内部 epoch，
防止旧 map 状态压制新配置的首个事件。证据 `cpu` 是 tracepoint 报告的 policy
CPU，不是 hook 执行 CPU；PID/TID 为零。该事件只证明 policy 最大值在变更时低于
配置下限，不证明瞬时频率、共享 policy 的完整 CPU 成员、限频原因或持续时间。

### 4.2 用户态观测点

用户态观测优先使用稳定的 ESOP 符号或显式 trace hook。Rust `async fn` 的普通
uprobe/uretprobe 只测得 future 构造，不覆盖 await 的实际运行时间，因此 Zenoh
gateway publish 与同步 callback 路径使用独立的显式版本化 C ABI：

```text
esop_zenoh_gateway_publish_begin_v1(request_id: u64, route_kind: u32)
esop_zenoh_gateway_publish_end_v1(request_id: u64, route_kind: u32, outcome: u32)
esop_zenoh_gateway_callback_begin_v1(request_id: u64, route_kind: u32)
esop_zenoh_gateway_callback_end_v1(request_id: u64, route_kind: u32, outcome: u32)
esop_linux_raw_port_operation_begin_v1(ifindex: u32, operation: u32)
esop_linux_raw_port_operation_end_v1(ifindex: u32, operation: u32, outcome: u32)
```

gateway 为 publish 与 callback 共用一套进程内非零原子 request ID。publish 在完成
key/payload 准入后、Session put 前调用 begin；成功、传输失败和 future cancellation
分别以固定 outcome 恰好调用一次 end。command subscription 与 raw/typed query 在
调用用户 callback 前调用独立 begin，正常返回为 completed，Rust unwind 由 guard
`Drop` 收口为 abandoned；typed query 的 decode、provider、encode 和同步 reply wait
都在同一观测窗口内。marker 不解析字符串、不分配、不等待内核响应；未附加探针时
仅保留固定参数的空调用。进程 abort 不伪造 callback end，遗留状态由固定容量 LRU
约束。Linux raw port 在合法 TX 的 `send(2)` 前后及每次非阻塞 `recv(2)` 前后调用
独立 marker；TX outcome 区分成功、系统调用错误和部分写，RX outcome 区分 frame、
empty、link-down 和其他错误。marker 不读取时钟，错误路径先保存 `errno`，guard 保证
每个 begin 最多一个 end。其余计划观测点仍包括：

1. `esop_ros2_control` 的 `read()`、`write()` 和 controller update 边界。
2. Zenoh gateway 的 IPC、序列化、permit 和 reconnect。
3. Linux RT port 的 cycle begin/end、RX drain、commit、prepare，以及驱动/NIC 边界。
4. recorder、配置工具和维护进程的启动、退出、阻塞和异常返回。

BPF 侧用固定 1024 项 LRU map，以 `{TGID, request_id}` 保存 begin 时间、策略
epoch、起始 TID、route 和操作类别。publish 只接受 State/Event/Diagnostic，callback
只接受 Command/Query；end 必须匹配进程、request、类别、route、允许的 outcome 和
epoch，并在所有已匹配分支删除状态。只有
`duration_ns > gateway_stall_threshold_ns` 才输出固定 96 字节
`UserZenoh/GatewayStall` 证据。`evidence_id` 保留 request ID，
`observed_value == duration_ns`，`detail` 的低 3 bit 为 route、高位为 outcome；若
async task 在另一 worker thread 完成，PID 保留、TID 置零。相关器还要求同 cycle
存在 deadline/WKC/DC 风险，不能只凭 enum 值生成 incident。

Aya runtime 只在调用方给出目标 ELF 后显式附加 publish 或 callback begin/end
程序。每组探针对独立原子生效：第二个附加失败会卸载第一个 link。可选模式返回
不可用并保留内核 tracepoint 基线；required 模式返回错误且不声明 readiness。所有
用户 hook 都必须携带固定的 `boot_id`、`cycle_seq` 或 `request_id`，不得解析动态
字符串；稳定符号保持 ABI 版本，缺失时等价于 `USER_PROBE_UNAVAILABLE`，不影响
RT 核心。

raw-port BPF 路径使用独立固定 1024 项 LRU map，以 `pid_tgid` 保存 begin 时间、
策略 epoch、ifindex 和 TX/RX operation。重复 begin 会替换同线程旧状态并增加 mismatch；
匹配 end 在校验 ifindex、operation、outcome 或 epoch 前先删除状态。只有
`duration_ns > raw_port_stall_threshold_ns` 才输出固定 96 字节
`UserEsop/RawPortStall` 证据，携带 TGID/TID、结束 CPU、ifindex、时长、阈值以及
operation/outcome detail。相关器还要求同 cycle 存在 deadline/WKC/DC 风险，才生成
`HOST_PORT_STALL`。这条证据只覆盖 raw socket 系统调用边界，不覆盖调用方调度、
驱动队列、NAPI/IRQ、NIC DMA、线缆、从站响应、WKC 或完整 EtherCAT 周期。

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
| `HOST_NIC_DROP` | 指定 EtherType/网卡的 `kfree_skb` 窗口达到计数阈值，且与 WKC/timeout/deadline 风险同窗。 | Linux 网络路径存在可归因的协议栈丢弃。 | 提供 `HOST_OBSERVATION`，不能替代 EtherCAT WKC。 |
| `HOST_PAGE_FAULT` | 受跟踪进程的 `{CPU, 进程}` 页错误窗口达到计数阈值，且与 deadline/WKC/DC 风险周期同窗。 | 周期可能被内存管理事件打断；当前证据不区分 major/minor。 | 性能资格失败或按策略撤销 host permit。 |
| `HOST_OOM` | `oom:mark_victim` 为受跟踪 PID 选择 OOM victim。 | 内核已把该任务标记为 OOM victim；证据不推断 TGID 或触发者。 | 作为 hard fact 锁存观测故障；仍不直接写 controlword。 |
| `HOST_CPU_THROTTLE` | cpufreq policy `max_freq` 低于产品配置下限，且与 deadline/WKC/DC 风险周期同窗。 | 内核 policy 上限不足以满足已配置频率前提；原因和持续时间未知。 | supervisor lease 降级。 |
| `HOST_PORT_STALL` | Linux raw port 的 `send(2)` 或非阻塞 `recv(2)` 调用严格超过独立阈值，字段自洽且与 deadline/WKC/DC 风险周期同窗。 | raw socket 系统调用边界与周期风险相关；不证明驱动、NIC、线缆、从站或整周期根因。 | 可请求普通 controlled stop；不直接写 controlword。 |
| `USER_COMPONENT_EXIT` | 受跟踪 gateway、ROS 2 controller、recorder 或 agent 的主线程退出。 | 用户态组件主生命周期异常；工作线程退出不升级。 | MLG 只根据固定 supervisor lease/command age 判定。 |
| `OBSERVABILITY_DEGRADED` | BTF/attach/permission/ringbuf/agent health 失败。 | 观测证据不完整。 | 不能自动声称健康；是否禁止运动由产品 policy 决定。 |

### 6.2 检测规则原则

1. 每条规则包含 `enter_threshold`、`exit_threshold`、`window_ns`、`min_count`、`max_age_ns` 和严重级别。
2. 内核程序只做固定容量计数/窗口聚合；跨证据关联、cycle 风险判断和事故合并由用户态相关器执行。
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
| EBPF-003 | 调度 | 人为注入线程延迟、CPU 迁移、IRQ/softirq 压力和 cpufreq policy 限制，能生成 `HOST_SCHEDULER_STALL`、`HOST_IRQ_STORM` 或 `HOST_CPU_THROTTLE` 证据。 |
| EBPF-004 | 网络 | 注入网卡队列、协议栈丢包和 raw-port syscall 延迟，能分别关联 `HOST_NIC_DROP`、`HOST_PORT_STALL` 与 EtherCAT WKC/timeout 窗口。 |
| EBPF-005 | 内存 | 注入页错误、内存压力和进程 OOM/退出，能生成 `HOST_PAGE_FAULT`/`HOST_OOM`/`USER_COMPONENT_EXIT`。 |
| EBPF-006 | 用户态 | gateway、`ros2_control`、recorder 重启或函数超时能按 PID/TID、cycle_seq 和 request_id 归因。 |
| EBPF-007 | 完整性 | agent 无法写 MLG、controlword、permit；伪造/过期 host observation 不可清除 fault latch。 |
| EBPF-008 | 降级 | ringbuf 满、事件丢失、agent 重启、BTF 缺失、权限不足时，丢失计数和 `OBSERVABILITY_DEGRADED` 可见。 |
| EBPF-009 | 相关性 | 生成包含 RT 事件、ProcBuf 状态、MLG transition 和 eBPF evidence ID 的 incident timeline。 |
| EBPF-010 | 开销 | baseline/incident/forensics 三档测得 CPU、内存、ringbuf、事件丢失和 host RT 影响；不改变 MCU 资格结论。 |
| EBPF-011 | 长测 | 至少 30 分钟 Q1/Q2 Linux 监督域压力测试，无 agent 内存增长、无无限 map 增长、无周期阻塞。 |
| EBPF-012 | 安全边界 | 产品测试报告明确 eBPF 不是安全通道；STO/FSoE/安全 PLC 仍独立验证。 |

EBPF-003 的调度迁移子路径已增加专用特权托管 Linux 资格。
`make test-ebpf-scheduler-migration-runtime` 从实际 allowed affinity 中选择两个 CPU，
仅挂载 `sched_migrate_task`，在精确跟踪同步 worker TID 后强制 A→B→A，并证明
第一次移动没有 ringbuf 记录、第二次移动产生唯一 `CpuMigration` 和
Warning/confidence-60 `HostSchedulerStall`，统计严格为两次迁移、一次阈值事件、
一次发出且零 loss，observer 从 Healthy 转为 fault `0x45422001` 的 Degraded。
报告写入 `build/ebpf_scheduler_migration_qualification.json` 并由封闭 schema 独立
校验。调度 runqueue 子路径还通过
`make test-ebpf-scheduler-runqueue-runtime` 完成真实 `sched_wakeup`/`sched_switch`
成对挂载、精确 futex waiter TID、CPU A 有界 FIFO 竞争、25 ms hold、唯一 Error
`SchedulerRunqueueLatency`/`HostSchedulerStall`/`ControlledStop` 和 Degraded
heartbeat 资格。自然负载 runqueue 根因、IRQ/softirq 压力、cpufreq policy 限制、
产品优先级/CPU 隔离、生产目标内核、迁移原因/亲和性/cache/NUMA、开销/WCET
和长测仍未资格化，因此 EBPF-003 整体保持 partial。

EBPF-003 的 softirq-duration 子路径还通过 `make test-ebpf-softirq-runtime` 完成真实
softirq pair 挂载、目标 CPU/vector 过滤、54-segment loopback UDP GSO 注入、独立
校准和正式阈值、唯一 `SoftirqCpuTime`/`HostIrqStorm`/`ControlledStop` 以及
Degraded heartbeat 资格。这里“仍未资格化的 IRQ/softirq 压力”明确指硬 IRQ、真实
NIC/driver/NAPI、持续 softirq 压力与产品预算，不再包含这条受控 hosted loopback
`NET_RX` 单次执行链；因此 EBPF-003 整体仍保持 partial。

EBPF-004 当前已具备有界 CPU/ifindex 窗口、EtherType/ifindex 过滤、drop
reason 证据解码和 transport-risk 相关器单元测试。该实现与 CO-RE 编译结果
不等于目标内核真实队列压力、丢包注入、verifier 和开销资格。raw-port 子路径
当前已具备稳定 v1 begin/end marker、TX/RX outcome、独立阈值/epoch、固定 1024 项
per-thread LRU、原子 uprobe pair、96 字节 `RawPortStall` 解码和风险周期相关器拒绝
条件。专用 `make test-ebpf-raw-port-runtime` 还会在特权托管 Linux 上关闭无关
tracepoint，加载真实 CO-RE 对象并通过 verifier，把完整 marker pair 精确附加到夹具
自身，以 WKC-risk cycle 包围一个 25 ms 延迟释放、阈值为 5 ms 的 Unix-domain
阻塞 `recv(2)`，再要求 ringbuf 解码、begin/completion/stall 统计和唯一
`HostPortStall` controlled-stop incident 全部自洽、零 mismatch/loss，最后生成并校验
`build/ebpf_raw_port_qualification.json`。该资格只覆盖共享 Unix-socket
marker-to-incident 路径；真实 AF_PACKET 非阻塞 RX/NIC 负载、生产目标内核、
驱动/NIC/线缆/从站归因、完整周期、开销/WCET 和长时 HIL 仍未资格化，因此
EBPF-004 整体保持 partial。

EBPF-005 当前已具备有界 CPU/进程页错误窗口、阈值事件、架构错误码 detail
解码和 cycle-risk 相关器单元测试，并已把进程退出限制为受跟踪 TGID 的主
线程、把 OOM 证据绑定到 `mark_victim` 的受害 PID。专用
`make test-ebpf-page-fault-runtime` 会在特权托管 x86_64 Linux 上只要求
`exceptions:page_fault_user`，用预热独立子进程对 16 个匿名页执行首次写入，
要求 `ru_minflt`、BPF 和 evidence 三组计数精确一致，并验证唯一 Warning
`HostPageFault`/`DegradeHostObservation`、零 loss 和 Degraded heartbeat。
`make test-ebpf-process-exit-runtime` 则加载真实 CO-RE 对象、只要求
`sched_process_exit`，证明一个受控 worker 退出被抑制且 observer 保持 Healthy，
再证明受跟踪 leader 正常退出生成唯一 `ProcessExit`、Critical
`UserComponentExit`/`LatchFault` 和 Failed heartbeat。两者分别严格校验
`build/ebpf_page_fault_qualification.json` 与
`build/ebpf_process_exit_qualification.json`。`make test-ebpf-oom-runtime` 另在新建
cgroup v2 叶子内限制一个预热单线程子进程，以 32 MiB `memory.max` 和有界
128 MiB 匿名页写入触发局部 OOM；夹具要求子进程由 `SIGKILL` 结束，
`memory.events.local` 至少一次 OOM、恰好一次 OOM kill 且零 group kill，并只生成
唯一 `KernelMemory/OomKill`、Critical `HostOom`/`LatchFault`、零 loss 和 Failed
heartbeat，严格报告写入 `build/ebpf_oom_qualification.json`。页错误资格不含 fault address/IP、
major/minor、handler duration、内存压力、swap/storage 或自然负载根因；退出资格
不含 exit code/signal，不证明线程组完全消失；OOM 资格只证明 hosted 单进程
memcg victim-PID 链，不证明 victim TGID、PID namespace/cgroup identity、触发者、
全局压力、容器委派或组件重启。生产目标内核、开销和长时 HIL 仍需单独资格化，因此
EBPF-005 整体保持 partial。

EBPF-006 当前已完成 Zenoh gateway publish 与 callback 子路径：独立稳定 v1
begin/end marker、共享非零 request ID、future cancellation 与 callback unwind 收口、
两组可选原子 uprobe 对、记录操作类别的固定 1024 项 epoch-aware LRU 状态、96 字节
request-correlated 证据解码和 transport-risk 相关器拒绝条件已有源码/单元测试。
command subscription 及 raw/typed query 的同步 callback 调用均已覆盖。专用
`make test-ebpf-gateway-runtime` 还会在特权托管 Linux 上关闭无关 tracepoint，加载真实
CO-RE 对象并通过 verifier，把 publish/callback 四个 marker 精确附加到夹具自身，以
同一 WKC-risk cycle 分别包围 25 ms 的 Diagnostic/success publish 和
Command/completed callback 延迟，再要求两条 ringbuf 证据合并为一个 count/evidence
count 均为 2 的 `GatewayStall` controlled-stop incident，且附着位、request ID、detail、
begin/completion/stall 统计全部自洽、零 mismatch/loss，最后生成并校验
`build/ebpf_gateway_qualification.json`。该资格只覆盖 direct-marker 到 incident 的共享
路径；live Zenoh Session、router/transport queue、IPC、序列化、permit、reconnect、
生产目标内核和开销仍未资格化，ROS 2 与 recorder hook 也未完成，因此 EBPF-006
整体保持 partial。

FR-048 的调度迁移路径除 typed 读取、独立 scheduler TID、固定迁移窗口、策略
epoch、来源/目标 CPU 解码和相关器单元测试外，现已具备上述 hosted-kernel
两 CPU 真实 attach/迁移/ringbuf/统计/incident/health 资格；runqueue 路径也已
具备精确 TID、真实 wakeup/switch pair、受控 FIFO 竞争和 measured
wake-to-switch/incident/health 资格；softirq 路径已具备 CPU/vector 过滤的受控
loopback `NET_RX` duration/incident/health 资格；page-fault 路径已具备预热独立
子进程、16 页匿名内存首次写入、精确 `ru_minflt`/BPF/evidence 计数和唯一
incident/health 资格；OOM 路径已具备独立 cgroup v2 单进程、局部事件计数、
`SIGKILL`、精确 victim PID、incident 和 health 资格。这些结果仍不等于生产目标内核、自然负载 runqueue 根因、
硬 IRQ 或真实 NIC/持续 softirq 压力、页错误内存压力/major-minor 归因、产品
优先级/CPU 隔离、全局 OOM/受害 TGID/namespace/cgroup 归因、迁移原因/亲和性/cache/NUMA 或开销资格。CPU 限频路径已具备
typed `cpu_frequency_limits`、policy CPU 过滤、
低于产品下限 episode 去重、固定事件解码和相关器单元测试，但真实 policy 限制
注入、共享 policy 拓扑、瞬时频率/驻留时间/原因归因、verifier 和开销资格仍未完成。

## 11. 运行时输出示例

一次周期异常的可解释链路应类似：

```text
cycle 184220: deadline_miss
  -> MLG transition_seq 882: HOST_OBSERVATION degraded
  -> USER_ESOP: raw-port send() syscall delayed 1.8 ms on ifindex 7
  -> KERNEL_SCHED: raw-port TID runqueue latency 1.2 ms on CPU 3
  -> KERNEL_IRQ: eth IRQ/softirq consumed 0.9 ms in same window
  -> KERNEL_NET: RX queue drop count +4 on eth0
  -> EtherCAT: Domain actual WKC < expected, input age +1
  -> action: revoke host lease, configured stop=QUICK_STOP, await recovery
```

该链路中的“事实”来自各自的原始事件；`CORRELATOR` 只说明它们在同一个 cycle/time window 内相关，不把相关性伪装成经过形式证明的单一根因。

## 12. 当前未决项

1. 量产 Linux 内核最低版本、是否强制 CONFIG_DEBUG_INFO_BTF、目标发行版及 libbpf 版本。
2. Linux raw port 的 cycle/RX-drain/prepare/commit 与驱动/NIC 边界、`esop_ros2_control`、recorder 及 Zenoh IPC/序列化/permit/reconnect 的稳定用户态 hook 名称与 ABI；raw-port send/recv、Zenoh publish 和 command/query callback v1 marker 已固定。
3. 关键 host gate 是否作为 split-linux-rt 的运动前提，以及其宽限期和停止策略。
4. 目标网卡的可观测 tracepoint、驱动特定 attach 点、RX/TX queue 映射与丢包口径。
5. baseline/incident/forensics 的采样率、数据留存时长、隐私字段和远程上传策略。
6. 生产环境 eBPF 加载权限、签名/完整性保护、systemd/cgroup 隔离和升级回滚流程。

## 13. 参考依据

- Linux Kernel Documentation：BPF verifier、BPF ring buffer、libbpf、BTF、program types。
- libbpf 官方文档与 `libbpf-bootstrap`：CO-RE 应用生命周期、uprobe 示例、ring buffer 与旧内核兼容思路。
- 本设计不把 eBPF 的存在、加载成功或观测结果当作功能安全认证证据；安全结论仍由独立安全需求、风险评估和验证流程给出。
