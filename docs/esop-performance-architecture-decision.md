# ADR-0001：ESOP 实时与性能架构决策

- 文档版本：1.0
- 日期：2026-09-03
- 状态：已接受，作为 ESOP EtherCAT 数据面实现与性能验收基线
- 产品全名：`ESOP = EtherCAT Simple Operating System`
- 相关文档：[EtherCAT 主站需求与架构说明](ethercat-master-requirements.md)、[机器人 ESOP 软件规划](robotics-esop-software-plan.md)
- 主要参考：IgH EtherCAT Master 官方仓库提交 `650888c587b1aa570c4d7211adaf39145d9e5ae3`，提交日期 2026-08-21

> 实现状态（2026-09-04）：已形成 `crates/esop-ethercat-core/` Rust `no_std` 基础 crate，覆盖调用方固定 arena、线缆帧编解码、固定帧池、O(1) RX index、有界收发周期、静态 PDO Frame Plan 到 Domain 的接收提交路径、多速率预计算调度、控制请求闭环、在线扫描、SII 身份与 SyncManager/RxPDO/TxPDO category 只读解析、固定容量 EEPROM 分块读取及事务式固定容量配置候选、按 PDO 类别分段的多 SyncManager FMMU 逻辑地址分配、SM/FMMU 映射校验与写入读回 FSM、固定容量 CoE PDO assignment/mapping 写序列、启动编排、固定二进制诊断事件环、SPSC ring、固定容量 DMA TX/RX 描述符所有权环与缓存维护契约、Frame Plan 到 DMA TX descriptor 的零中间帧拷贝构建和提交端口契约、DMA RX descriptor 直接消费会话、Mailbox 轮询 FSM、有限预算重试与协议错帧恢复、可配置 Status Bit 邮箱轮询、CoE SDO expedited/segmented 事务 FSM、异步 Emergency 固定事件环，以及 DC SYNC0/SYNC1 配置 FSM、FRMW reference-clock 周期同步槽和 offset/jitter 监测器；独立 `esop-profile-cia402` crate 已具备 Statusword FSA 解码、基础 Controlword 使能序列、生命周期拒绝、Fault reset 单脉冲、CSP/CSV/CST 模式监督和基于核心 `PdoEntry` 的标准周期 PDO typed raw binding；`esop-ethercat-linux-port` 已具备 Linux AF_PACKET 端口和固定容量确定性 `SimulatedPort`。通用 completion queue、真实 MCU/Linux DMA 端口适配、完整 SII/ESI PDO 自动发现、真实从站互操作、单位/缩放与厂商 quirk、DC 拓扑/全从站运行时同步和跨任务 ProcBuf 原子交接仍未完成，不能对外宣称“已实现完整主站”或“已实现持续无锁通信”。

> 追加实现状态（2026-09-04）：`crates/esop-procbuf` 已提供固定布局 Header/layout hash、Command/State 双页原子快照、Quality/Lifecycle/Runtime observation 数据和固定容量事件环。shared-memory/RPMsg/UDS IPC 适配仍属于集成层工作。

> 追加实现状态（2026-09-04）：`esop-ethercat-core` 已增加固定容量 `DomainRegistry`，统一登记多 Domain、PDO entry、datagram 与周期/相位，激活前完成过程映像和逻辑地址冲突检查，激活后锁定注册，并可生成现有 `Domain` 输入段与 `FramePlan` 片段。

> 追加实现状态（2026-09-04）：SII 配置候选现在可冻结为 `SiiDomainProjection`，在进入 `DomainRegistry` 前校验 Rx/Tx 统一映像偏移、FMMU/SyncManager 物理范围和逻辑基址；注册表更新采用固定容量副本提交，映像或容量不足时不发布半配置。

> 追加实现状态（2026-09-04）：`FramePlanSet` 可在固定帧槽内按 datagram 容量和 Ethernet MTU 拆分计划；字节对齐的 SII segment 可自动生成方向正确的 `LWR`/`LRD` datagram。多帧计划与 `DomainRegistry` phase 通过临时副本一起校验，失败不发布部分计划。

## 1. 决策摘要

当前 SII 配置路径已包含固定容量的分块读取、类别投影和候选配置原子发布编排；它仍不是完整的 ESI/从站自动发现流程。

ESOP 采用 IgH 已验证的调用者驱动周期、Domain、外部过程数据内存、帧聚合、轮询收包、异步请求、DC 预分配和 acquire/release 状态交接思想，但不移植 IgH 的 Linux 内核架构、运行期链表调度、周期内线性数据报匹配、字符设备、`ioctl`、`mmap`、通用 socket 驱动或可能重新分配的请求缓冲。

ESOP 的性能架构固定为：

1. 配置期可做复杂计算，激活后配置冻结，所有实时对象来自调用方提供的静态 arena。
2. 主站不创建内部实时线程；调用方按固定周期执行 `receive -> validate/commit -> control -> prepare/queue -> send`。
3. 激活阶段生成不可变的多速率调度表、Frame Plan、连续 Slot 数组和 256 项 RX 索引表。
4. TX 首版采用“DMA descriptor 缓冲内直接构帧 + 一次有界 payload 拷贝”，不为了名义零拷贝引入复杂且不可移植的 scatter-gather 依赖。
5. RX 在 DMA staging 中完成长度、类型、索引、地址、WKC 和周期归属校验，只有完整合格的数据才提交到 ProcBuf State page。
6. PDO、DC 和控制面使用不同优先级与独立预算；控制面不得等待、挤占或延迟 P0 周期数据。
7. 热路径只写计数器、时间戳和固定事件记录；JSON、Protobuf、Zenoh、ROS 2、文本日志和文件 IO 全部位于非实时域。

本决策的目标不是在未测试前宣称“绝对性能优于 IgH”。ESOP 的目标是：在 MCU/嵌入式场景中依赖更少、执行路径更短且有界、配置冻结后零堆分配，并通过相同硬件、拓扑、周期和 PDO 负载下的基准测试证明具体性能。

## 2. 背景与问题

IgH 的设计服务于 Linux 内核主站和完整运维生态。它提供成熟的 Domain、FSM、周期 API 和原生轮询网卡驱动，但也需要 Linux 内核对象、网络设备、字符设备、用户态 `ioctl`/`mmap` 边界及大量通用性代码。

ESOP 的首要运行环境是 STM32、HPMicro 和其他具有片上 Ethernet MAC/DMA 的 MCU，也要保留 Linux 开发与 HIL 端口。因此性能问题不是单纯追求平均吞吐，而是同时满足：

- 每个周期的工作量和最坏执行路径可解释、可测量；
- 无周期内堆分配、阻塞锁、睡眠和无限轮询；
- DMA、D-cache、ISR 与实时任务之间有明确所有权；
- 多 Domain、CoE、DC 和诊断不会破坏伺服 PDO 的截止时间；
- 过程数据不因半帧、旧帧、重复帧或 WKC 异常被部分更新；
- 同一协议核心可在 Cortex-M、RISC-V MCU 和 ARM/Linux 上编译验证。

## 3. 目标与非目标

### 3.1 目标

| 目标 | 说明 |
| --- | --- |
| 确定性 | 激活后周期路径只遍历固定数组和固定上限循环，禁止与从站总数无关的隐藏阻塞。 |
| 低抖动 | 使用硬件定时器、轮询/有界 RX drain、预计算计划和无等待交接，减少 ISR、锁和动态调度抖动。 |
| 数据完整性 | 只有完整匹配且质量合格的 RX 结果才能原子发布到正式状态页。 |
| 可移植 | 协议核心为 Rust `no_std`；平台差异限制在 timer、DMA、cache、IRQ、原始二层收发和原子能力。 |
| 可测量 | 每个发布构建输出资源、线缆预算、执行时间分布、抖动、WKC、超时和副本统计。 |
| 可降级 | 负载过高时先延迟/拒绝控制面，再降采样低优先级 Domain，绝不静默挤占运动 Domain。 |

### 3.2 非目标

- 不复刻 IgH 的 Linux 内核模块、私有网卡驱动 fork、RTDM、字符设备或 CLI。
- 不在首版实现双网口冗余、在线任意重映射或动态拓扑无缝恢复。
- 不承诺任意板卡、PHY、RTOS 和从站组合都达到 250 us。
- 不把 AF_PACKET、TCP/IP 栈或普通 socket 路径的结果当成 MCU DMA 端口性能代表。
- 不让未验证的 RX DMA 数据直接覆盖正式 ProcBuf 输入。
- 不以平均值代替 P99、最大值、deadline miss、WKC 和超时验收。

## 4. IgH 性能机制与 ESOP 处理

以下路径均相对于 IgH 官方仓库提交 `650888c587b1aa570c4d7211adaf39145d9e5ae3`。

| IgH 机制 | 证据位置 | ESOP 结论 |
| --- | --- | --- |
| 主站在激活后由应用周期驱动 | `README.md:74-79`；`include/ecrt.h:1042-1057` | **采用。** ESOP 不创建内部 RT 线程，调度器由产品控制。 |
| 显式 `receive`/`send` RT API | `include/ecrt.h:1101-1133` | **采用并细化。** 拆分为有界 receive、validate/commit、control、prepare、send 阶段。 |
| Domain 可直接返回过程映像并支持外部内存 | `include/ecrt.h:2183-2221`；`master/domain.c:433-452` | **采用并加强。** ProcBuf/Domain 使用生成式固定布局，但 RX 先写 staging，验证后提交。 |
| 多 Domain 支持不同采样率 | `FEATURES.md:37-41` | **采用。** 激活时生成 hyperperiod 槽表，周期内不计算模除调度。 |
| 配置/激活阶段分配，周期 API 标记 RT safe | `include/ecrt.h:748-762`、`1042-1057` | **改造。** ESOP 所有内存由调用方 arena 提供，激活后分配器永久关闭。 |
| 激活时计算 FMMU、逻辑地址、数据报及 expected WKC | `master/domain.c:225-324` | **采用。** 同阶段额外生成帧布局、RX 索引表、预算报告和不可变校验摘要。 |
| 在 MTU 内聚合多个数据报 | `master/master.c:996-1123` | **采用并优化。** 用预计算 Frame Plan 替代周期内链表扫描和 fit 判断。 |
| 直接取得设备 TX 缓冲构帧 | `master/master.c:1033-1095` | **采用。** MCU 端直接在可用 DMA TX descriptor 缓冲内写头、payload 和 WKC 初值。 |
| 原生驱动无中断轮询 | `FEATURES.md:9-18`；`master/device.c:492-514` | **采用思想。** 定时器释放 RT 任务，ISR 只确认 DMA/置位；RT 任务有界 drain RX。 |
| generic 驱动经 PF_PACKET/socket | `devices/generic.c:203-250`、`312-360` | **仅兼容。** Linux 开发/HIL 可用，不得作为产品性能基线。 |
| 控制面数据报使用固定 32 项环 | `master/master.h:109-113`；`master/master.c:796-905` | **采用模式并加强。** 使用配置化固定池、优先级队列及 byte/datagram/time 三重预算。 |
| 根据 send interval 计算最大排队字节并保留 10% | `master/master.c:909-921` | **改造。** 使用完整线缆占用、传播、DMA、软件执行和安全余量的离线预算。 |
| SDO 请求异步调度并暴露状态 | `include/ecrt.h:2271-2419` | **采用。** 请求来自固定池；最大响应长度固定，超长返回 `NO_SPACE`，不得重新分配。 |
| acquire/release 交接 FSM 与 RT 状态 | `master/smp.h:34-50`；`master/master.c:2452-2458` | **采用。** 使用 C11 原子或平台等价屏障，禁止用 `volatile` 代替同步。 |
| 非阻塞发送扩展队列 | `master/master.c:2547-2561` | **采用语义。** RT 不等待控制面锁；使用 SPSC ring 或 try 操作，失败返回 `BUSY/DEFERRED`。 |
| DC 数据报预分配，周期中更新值后排队 | `master/master.c:2800-2889` | **采用。** DC 槽编入 Frame Plan，频率、相位和位置在激活时确定。 |
| 每 Domain 统计 expected/actual WKC | `master/domain.c:457-643`、`679-702` | **采用并扩展。** 增加连续异常、last valid cycle、input age 和 commit 状态。 |
| 用户态过程映像通过激活后 `mmap` 获得并预触页 | `lib/master.c:559-608` | **不移植边界。** MCU 直接使用 arena；Linux RT 端口在启动期锁页、预触页，周期内无 ioctl。 |
| TX 对每个数据报复制 payload | `master/master.c:1069-1071` | **首版接受一次有界复制。** 后续只在证据表明必要时引入 SG DMA。 |
| RX 在线性链表中按 index/type/size 查找 | `master/master.c:1205-1215` | **拒绝。** 使用 256 项 `index -> slot` 表做 O(1) 候选定位，再校验 generation/type/size。 |
| RX 再复制到数据报内存 | `master/master.c:1238-1244` | **改造。** 解析到固定 staging，按 Domain commit plan 一次提交所需输入字段。 |
| Domain queue/process 周期内遍历链表 | `master/domain.c:477-563`、`648-674` | **拒绝热路径链表。** 激活后只使用连续数组和索引。 |

## 5. 总体性能架构

```text
                         non-realtime control/observability plane
  ROS 2 / Zenoh / Protobuf / CLI / recorder / configuration generator
                    | SPSC command/request | snapshot/event ring
--------------------+----------------------+-----------------------------
                         hard realtime ESOP plane
  timer release
      -> bounded RX DMA drain
      -> O(1) slot match + frame validation
      -> per-domain WKC validation + input commit
      -> command snapshot + robot control/profile step
      -> execute precomputed schedule slot
      -> build into TX DMA descriptors from Frame Plan
      -> cache clean + DMA ownership handoff
--------------------+----------------------------------------------------
                         port boundary
  monotonic timer | IRQ ack | atomics | D-cache | TX/RX descriptors | PHY
```

激活前对象可以是便于配置的图、列表和生成器描述；激活成功后，运行时只持有压平后的只读计划和固定状态数组。调试构建必须能校验只读计划的 hash，发现运行期意外修改立即进入 fault。

## 6. 详细决策

### PERF-D001：单 arena 与激活冻结

调用方先调用 size-query API 得到各内存区的大小和对齐，再提供内存：

| 内存区 | 生命周期 | 典型内容 |
| --- | --- | --- |
| `core_arena` | init 到 deactivate | master、slave、Domain、FSM、请求池、状态和统计 |
| `plan_arena` | activate 到 deactivate，只读 | Frame Plan、Slot、schedule、commit plan、expected WKC、配置 hash |
| `procbuf_region` | 产品运行期 | Command、State、Quality、event ring |
| `dma_region` | port 运行期 | TX/RX descriptor、帧缓冲、所有权状态 |
| `trace_region` | 可选 | 固定大小二进制 trace records |

`esop_activate()` 必须完成所有容量检查、地址分配、帧拆分、索引预留和预算验证。成功后设置 `allocation_closed = true`；任何核心分配入口被再次调用都返回 `ESOP_E_FROZEN`，调试构建同时计数或断言。

### PERF-D002：调用者驱动的五阶段周期

主站不持有线程。产品调度器提供绝对时间周期释放，并按固定次序调用：

1. `esop_cycle_receive()`：有界读取本周期前已返回的 RX descriptor。
2. `esop_cycle_commit()`：校验 slot/帧/Domain 并发布合格输入。
3. `robot_control_step()`：读取一致命令和输入，执行 profile/控制策略。
4. `esop_cycle_prepare()`：选择当前调度槽，更新 PDO、DC 和允许的维护数据报。
5. `esop_cycle_send()`：在 TX DMA 缓冲构帧并提交 descriptor。

默认采用跨周期流水：周期 `N` 开始处理周期 `N-1` 的返回帧，周期 `N` 末尾发送新的帧。若某硬件/控制算法要求同周期 send-then-wait-receive，必须作为独立模式建模，等待仍需有严格时间上限，且不得作为默认实现。

### PERF-D003：预计算 Frame Plan

激活阶段为 hyperperiod 的每个调度槽生成 `frame_plan[]`：

```text
schedule_slot
  due_domain_mask
  control_budget
  frame[first..count]
    tx_length, expected_rx_length, deadline_offset
    datagram[first..count]
      cmd, index, address, length, payload_source, expected_wkc
```

运行期不再进行链表遍历、装箱搜索、MTU fit 判断或动态 index 分配。计划生成器必须满足：

- 单帧 EtherCAT payload 和 Ethernet 帧长合法；
- 每个 index 在其有效 generation 内唯一；
- P0 PDO 顺序稳定且不被控制面改变；
- DC 插槽、Domain WKC 和 RX commit 范围可追踪；
- 计划总线占用低于配置的 wire budget；
- 同一静态配置生成字节一致的计划摘要。

### PERF-D004：Domain、ProcBuf 与安全提交

ProcBuf 是应用语义 ABI，Domain 是 EtherCAT 逻辑映像。生成器允许二者共享字段 offset，但不允许 RX DMA 在校验前获得 State page 的写权限。

每个 Domain 使用：

- `output_view`：RT 任务拥有，prepare 阶段读出并写入 TX；
- `input_staging[2]`：RX parser 拥有，按 frame generation 轮换；
- `input_committed`：控制器/State page 读取的最后完整输入；
- `quality`：expected/actual WKC、last valid cycle、age、fault counters。

只有下列条件全部满足才执行 commit：

1. Ethernet/EtherCAT 帧长度合法；
2. 每个数据报 index、generation、type、address 和 length 匹配计划；
3. 属于该 Domain 的所有必需数据报在 deadline 前到达；
4. actual WKC 满足 Domain 策略；
5. 没有重复提交、旧周期或 staging 溢出。

失败时保留上一份 committed input，增加 age，发布质量失败，按机器人策略 hold、ramp-to-zero 或 disable。禁止把“部分新输入 + 部分旧输入”伪装成一个有效状态。

### PERF-D005：O(1) RX Slot 匹配

EtherCAT index 为 8 位。ESOP 为每个活动端口保留固定 256 项表：

```c
typedef struct {
    uint16_t slot_id;
    uint16_t generation;
    uint16_t expected_size;
    uint8_t expected_type;
    uint8_t state;
} esop_rx_index_entry_t;
```

解析时先以 `index` 直接取候选项，再校验 generation、状态、type、size 和地址。index 不得在旧 slot 超时或完成前重用；当并发在途数可能超过可用 index 时，激活失败，而不是运行期退化为搜索。

### PERF-D006：DMA TX 与副本策略

当前核心已提供 `build_and_arm_dma_frame_from_plan()` 与 `submit_dma_frame()`：Frame Plan 直接写入 DMA ring 的 TX frame storage，RX index 在同一调用中绑定 descriptor generation，端口提交失败时回收 CPU/DMA ownership。该接口仍是通用端口契约，不等于某个 STM32/HPMicro MAC 已完成适配或实板资格验证。

首版 TX 采用确定性的一次构帧：

- Ethernet/EtherCAT/datagram 头在 descriptor 缓冲内原位写入；
- 固定字段和 WKC 初值原位写入；
- 过程数据按 plan 中的连续 copy span 拷入 payload；
- 对相邻 span 在激活时合并，降低 `memcpy` 调用数；
- 完成后只 clean 实际 TX cache range，再把 descriptor 交给 DMA。

不在首版实现通用 scatter-gather 零拷贝。只有当 benchmark 证明 payload copy 是 P99 热点，且所有量产 MAC 都能稳定支持所需 descriptor 链时，才可新增可选 SG port capability；Frame Plan 的语义不能因此改变。

### PERF-D007：有界轮询和 DMA 所有权

每周期 RX drain 同时受三项上限约束：`max_frames`、`max_bytes`、`max_time_ns`。任一预算耗尽即退出并记录原因，不继续忙等。

所有权状态固定为：

```text
RX: DMA_OWNED -> CPU_READY -> CPU_PARSING -> DMA_OWNED
TX: CPU_FREE  -> CPU_BUILD -> DMA_READY   -> DMA_OWNED -> CPU_FREE
```

ISR 只能确认中断源、更新 descriptor 完成位/单调计数、记录硬件错误并选择性唤醒 RT 任务。ISR 不解析 EtherCAT、不运行 CoE/FSM、不调用应用回调、不格式化日志。

### PERF-D008：数据面与控制面三重预算

发送优先级：

| 优先级 | 数据 | 调度规则 |
| --- | --- | --- |
| P0 | 运动/安全相关 PDO、必要 DC | 预留固定 slot；不得被其他流量推迟。 |
| P1 | 低速 IO Domain、周期状态监控 | 预计算多速率 slot；过载时可按声明策略降频。 |
| P2 | CoE SDO、寄存器、扫描/恢复 FSM | 只使用 P0/P1 后的显式剩余预算，可延迟或拒绝。 |
| P3 | trace dump、原始抓包、扩展诊断 | 不进入硬 RT 发送计划，或仅在维护模式启用。 |

控制面每周期同时满足：

- `control_datagram_count <= budget_datagrams`；
- `control_wire_bytes <= budget_bytes`；
- `service_exec_time <= budget_time_ns`。

任一不足时请求保持 `DEFERRED`；超过请求 deadline 则变为 `TIMEOUT`。P2 队列不得因为头部大请求阻塞后续可执行小请求，应按优先级和 deadline 扫描固定上限候选项。

### PERF-D009：固定异步请求池

SDO、寄存器和后续邮箱协议使用固定请求池。每个请求在创建时确定最大 TX/RX 长度；运行时状态为 `FREE -> QUEUED -> BUSY -> SUCCESS/ERROR/TIMEOUT -> FREE`。

- 读响应超过容量时返回 `ESOP_E_NO_SPACE` 并报告 required size；
- BUSY 请求不得修改 index、subindex、buffer 或 timeout；
- 请求完成通过原子状态或 SPSC completion ring 发布；
- 周期任务不等待调用方消费结果；
- 大文件/FoE 只允许维护模式和独立预算，不能占用 P0 请求池。

### PERF-D010：多 Domain 多速率调度

所有 Domain 周期必须是 base tick 的整数倍。激活时计算有限 hyperperiod 并生成 schedule table：

```text
base tick = 250 us
Domain A  = 250 us  -> every slot
Domain B  = 1 ms    -> slots 0,4,8,...
Domain C  = 4 ms    -> slots 0,16,32,...
```

若最小公倍数导致 schedule table 超过配置上限，激活失败并要求调整周期；运行期不得用无界日历队列。每个 Domain 的相位可配置，以错开低速 Domain 和控制面峰值。

### PERF-D011：DC 插槽与时间路径

参考钟同步、从站时钟同步和 monitor 数据报全部预分配并写入 Frame Plan。运行期只更新 application time 或清零固定 payload。

- application time 来自单调高分辨率时钟，不来自 wall clock/ROS time；
- 固定 FRMW reference-clock 数据报可与 PDO 共帧或单独成帧，由离线预算决定，运行期只更新 process image 中的 application time；
- DC monitor 可低于 PDO 频率，但频率和相位必须显式；
- 记录 `dc_offset_ns`、`dc_jitter_ns`、last sync cycle 和失锁次数；
- DC 失锁不能静默继续高精度运动模式。

### PERF-D012：原子、内存屏障和 cache line

跨上下文共享状态使用 `_Atomic` 和 acquire/release，或由 port 声明等价实现。`volatile` 只用于 MMIO，不提供线程或 DMA 同步语义。

- producer 填完 page/descriptor 后 release 发布 sequence/owner；
- consumer acquire 读取 sequence/owner 后访问内容；
- ISR/任务 ring 使用单写者单读者 head/tail；
- 高频写入计数器按 cache line 分隔，避免 RT 与非 RT 核之间 false sharing；
- DMA ownership 切换前后执行平台要求的 clean/invalidate 和设备屏障；
- ProcBuf header 记录 ABI、layout hash、boot ID 和 cache line size 假设。

32 位 MCU 上的 64 位时间戳若非 lock-free，使用 sequence counter 快照或短临界区，不假定天然原子。

### PERF-D013：诊断离开热路径

热路径允许：饱和计数器、min/max、固定直方图 bucket、时间戳和固定 32/64 字节事件记录。禁止：`printf`、`snprintf`、JSON、Protobuf、符号解析、文件写入和动态字符串。

P50/P90/P99/P99.9 由非实时任务从直方图或原始 trace 计算。事件 ring 溢出时覆盖策略由配置指定，并始终保留 `lost_event_count`。

### PERF-D014：ROS 2、Zenoh 与 Protobuf 隔离

`esop_ros2_control`、`esop_zenoh_gateway` 和 Protobuf runtime 只能读取 State/Quality 快照、写 Command page 或投递固定 IPC 请求。它们不能：

- 直接持有 MAC/DMA descriptor；
- 调用 EtherCAT cycle API；
- 获得 P0 Domain 写锁；
- 让 ROS executor、router、网络或磁盘完成时间进入 PDO deadline；
- 在 RT 固件链接图中引入其运行库。

### PERF-D015：过载与故障确定性

过载处理顺序固定为：

1. 拒绝 P3 诊断流量；
2. 延迟 P2 邮箱/维护请求；
3. 按预声明策略降低 P1 Domain 频率；
4. 若 P0 仍无法在预算内执行，保持/撤销输出并进入 `CYCLE_OVERRUN` fault；
5. 禁止通过无限增加轮询时间、静默丢弃 P0 或继续发布无效 State 掩盖问题。

### PERF-D016：配置生成和构建证据

`esop_cfggen` 必须输出：

- Domain 逻辑地址、PDO offset、Frame Plan 和 schedule table；
- 每帧 wire bytes、估算线缆时间、expected WKC 和 deadline；
- arena、ProcBuf、DMA、stack、`.text`、`.rodata` 预算；
- 运行期上限：从站、Domain、frame、slot、请求、trace；
- 配置 hash 和用于固件/网关一致性检查的 layout hash；
- 机器可读 `robot_build_report.json`。

## 7. 周期执行伪代码

```c
void esop_rt_tick(esop_runtime_t *rt, uint64_t scheduled_ns)
{
    uint64_t start_ns = rt->port->time_now_ns(rt->port_ctx);
    esop_cycle_token_t token = esop_cycle_begin(rt, scheduled_ns, start_ns);

    esop_rx_budget_t rx_budget = {
        .max_frames = rt->limits.rx_frames_per_tick,
        .max_bytes = rt->limits.rx_bytes_per_tick,
        .deadline_ns = start_ns + rt->limits.rx_budget_ns,
    };

    esop_cycle_receive(rt, &token, &rx_budget);
    esop_cycle_commit(rt, &token); /* only complete and valid domains publish */

    esop_command_snapshot_t command;
    esop_procbuf_command_acquire(rt->procbuf, &command);
    robot_control_step(rt->robot, &command, esop_cycle_inputs(&token));

    const esop_schedule_slot_t *slot = esop_schedule_current(rt, token.number);
    esop_cycle_prepare(rt, &token, slot);
    esop_service_budgeted(rt, &token, &slot->control_budget);
    esop_cycle_send(rt, &token, slot);

    esop_cycle_end(rt, &token, rt->port->time_now_ns(rt->port_ctx));
}
```

调用约束：同一 `esop_runtime_t` 只有一个 RT owner。非 RT 线程只能通过 ProcBuf、SPSC ring 或只读 snapshot 交互，不能并发进入 cycle API。

## 8. 内存所有权与副本预算

### 8.1 所有权表

| 对象 | 写者 | 读者 | 交接方式 |
| --- | --- | --- | --- |
| Command page | supervisor/controller | RT task | 双页 + sequence release/acquire |
| State/Quality page | RT task | supervisor/ROS/Zenoh gateway | 双页 + sequence release/acquire |
| Domain output | RT control/profile | TX builder | 同一 RT 上下文，无锁 |
| RX DMA buffer | MAC DMA / RT parser | RT parser / MAC DMA | descriptor owner + cache invalidate |
| Input staging | RX parser | Domain commit | 同一 RT 上下文 |
| Input committed | Domain commit | RT control / State publisher | 指针或页索引切换 |
| Maintenance request ring | non-RT producer | RT service consumer | SPSC release/acquire |
| Completion/event ring | RT producer | non-RT consumer | SPSC release/acquire |
| Frame Plan | activate/config | RT task | 激活后只读，hash 校验 |

### 8.2 首版每周期允许副本

| 路径 | 允许副本 | 原因与上限 |
| --- | --- | --- |
| Command page -> RT snapshot | 1 次固定大小快照或稳定页引用 | 防止 supervisor 写到一半；大小由 layout 固定。 |
| Domain output -> TX DMA payload | 每个合并 copy span 1 次 | 保持 MAC 可移植性；总字节等于当周期输出 payload。 |
| RX DMA -> input staging/committed | 每个有效输入 span 最多 1 次提交 | 先校验后发布，避免污染正式输入。 |
| committed input -> State page | 若布局相同则页切换；否则 1 次生成式字段拷贝 | 由 cfggen 报告实际 copy bytes。 |
| State page -> Protobuf/ROS | 非实时域任意 | 不计入 EtherCAT 周期预算。 |

构建报告和性能报告必须分别给出 `tx_copy_bytes_per_cycle`、`rx_copy_bytes_per_cycle`、`copy_span_count` 和实际测量时间。不能只以“零拷贝”标签描述实现。

## 9. 时间与线缆预算

### 9.1 Ethernet 线缆占用

对不带 VLAN 的 100 Mbit/s Ethernet，单帧估算：

```text
mac_frame_bytes  = max(64, 14 + ethercat_payload_bytes + 4)
wire_bytes       = 8 + mac_frame_bytes + 12
wire_time_ns     = wire_bytes * 8 * 1e9 / 100000000
```

其中 8 B 表示 preamble + SFD，12 B 表示最小 inter-packet gap，64 B 包含从目的 MAC 到 FCS 的最小 MAC 帧。若端口使用 VLAN、额外 tag 或硬件有不同 accounting，port 必须覆盖公式参数，不能继续使用默认值。

一个周期的发送准入条件：

```text
T_wire = sum(frame_wire_time)
T_bus  = T_wire + sum(slave_forward_delay) + cable_propagation
T_path = T_release_jitter
       + T_rx_dma_and_cache + T_parse_and_commit
       + T_control
       + T_prepare_and_copy + T_tx_dma_and_cache
       + T_bus + T_guard

T_path <= cycle_period
```

线缆预算由 cfggen 做保守估算，最终以真实拓扑上的 TX/RX 硬件时间戳或 GPIO trace 校准。从站传播延迟、PHY、线缆和返回路径不能被 payload 字节公式替代。

### 9.2 周期预算分配

默认只作为初始门槛，产品可收紧但不能无证据放宽：

| 预算项 | 1 ms 基线 | 500 us P0 | 250 us stretch |
| --- | ---: | ---: | ---: |
| release jitter guard | 10% | 10% | 10% |
| RX drain + parse + commit | 15% | 18% | 20% |
| robot control/profile | 25% | 25% | 25% |
| prepare + TX submit | 10% | 12% | 15% |
| wire/topology | 按实际计算 | 按实际计算 | 按实际计算 |
| unallocated safety guard | 至少 20% | 至少 15% | 至少 10% |

若实际 wire/topology 占用使总预算超过周期，配置必须在 activate/cfggen 阶段失败。不得依靠运行时“尽量发完”。

### 9.3 测量定义

- `release_jitter_ns = actual_tick_start - scheduled_tick_start`，同时报告有符号和绝对值分布；
- `fast_path_ns = cycle_end - cycle_begin`，不含非实时报告生成；
- `rx_latency_ns = rx_descriptor_ready - tx_descriptor_handoff`，硬件支持时使用时间戳；
- `input_age_cycles = current_cycle - last_successful_commit_cycle`；
- `deadline_miss` 指 RT tick 或 P0 frame 超过绝对 deadline，不以是否最终收到帧为准；
- 最大值必须来自完整原始样本或无丢失 max accumulator，不能从抽样日志推断。

## 10. 平台优化策略

### 10.1 STM32 端口

- descriptor 和 DMA buffer 按具体 MAC 要求对齐并放入 DMA 可访问 RAM；链接脚本显式定义 section。
- 优先使用 MPU non-cacheable 区域存放 descriptor；payload 若为 cacheable，按精确 cache line clean/invalidate。
- TX/RX descriptor ring 静态创建；禁止 HAL 在周期中分配或复制整帧到内部临时区。
- 定时器以绝对周期释放最高优先级 RT 任务；Ethernet ISR 只确认完成和错误。
- 使用 DWT cycle counter 或高分辨率通用定时器测量阶段耗时；处理 32 位计数器回绕。
- 明确 SRAM bank 竞争、DMA bus master 和 CPU/D-cache 的放置，HIL 中同时施加其他 DMA 压力。
- HAL、LL 或厂商驱动版本属于端口实现细节，不得出现在协议 core 头文件。

### 10.2 HPMicro/RISC-V 端口

- 根据具体 SoC 的 GMAC、cache line、LMEM/AXI SRAM 和 DMA 可达性生成链接布局，不假定与 Cortex-M 相同。
- descriptor ownership 前后使用 RISC-V/SDK 所需的设备内存屏障和 cache API。
- PLIC 中断优先级低于周期 timer 的释放保证；ISR 同样只确认/置位。
- 使用 machine timer、GPTMR 或可验证的单调硬件时基，记录频率和换算误差。
- 对可配置 cache 或 non-cache 区分别 benchmark，选择 P99 更稳定的布局，而不是只比较平均带宽。
- 测试 RV32 上 64 位原子与时间戳快照实现，禁止把非 lock-free `_Atomic uint64_t` 隐式链接到不可控锁实现。

### 10.3 ARM/Linux 端口

- 开发端口可使用 AF_PACKET/raw socket；其 syscall、网络栈和调度抖动结果仅用于功能/HIL。
- 性能资格端口必须锁定内存并预触所有页，周期中禁止 page fault、`ioctl`、`mmap`、日志和配置调用。
- RT 线程使用绝对时间唤醒、固定 CPU affinity 和适用的实时调度策略；IRQ affinity、频率调节和电源状态记录进报告。
- 若采用 PACKET_MMAP、AF_XDP、专用驱动或其他加速端口，必须作为独立 port capability 和依赖声明，使用相同 benchmark schema 对比。
- Linux 结果不能代替 STM32/HPM 的 DMA cache、总线竞争和 ISR 验收，反之亦然。

### 10.4 所有平台共同规则

- release 构建打开合理编译优化和 LTO 与否必须记录；不得用不同编译参数比较实现。
- 热路径禁止未界定的除法、浮点格式化和大结构隐式拷贝；必要 64 位除法尽量移到激活期。
- 对 frame、slot、Domain 和请求数组使用连续内存；按访问顺序排列字段，冷诊断字段与热状态分离。
- 分支预测提示只能作为微优化，不能代替算法有界性。
- 任何汇编、SIMD 或平台 intrinsic 优化都必须保留 Rust 参考实现和等价测试。

## 11. 性能目标矩阵

### 11.1 资格场景

以下是支持等级，不是对所有硬件的无条件承诺。每个平台必须逐项发布通过/失败和具体限制。

| 场景 | 周期 | 轴/从站负载 | PDO 过程映像 | 帧计划 | 发布级别 |
| --- | ---: | --- | ---: | --- | --- |
| Q1 基线 | 1 ms | 32 轴 + 低速 IO，最多 32 从站 | <= 1024 B | 最多 2 个 P0/P1 帧，DC 开启 | P0 必须 |
| Q2 伺服 | 500 us | 16 轴，最多 24 从站 | <= 512 B | P0 单帧优先，DC 开启 | P0 必须 |
| Q3 高频 | 250 us | 8 轴，最多 16 从站 | <= 256 B | 单帧，控制面默认关闭，DC 可选 | stretch，不阻塞首发 |
| Q4 混合压力 | 1 ms | 16 轴 + IO + 8 个并发 SDO 请求 | <= 768 B | 多 Domain + P2 预算 | P0 隔离验收 |

轴负载必须给出实际每轴 RxPDO/TxPDO 字节数，不能只写“8/16/32 轴”。超过表中映像或帧数的产品配置重新计算预算和支持等级。

### 11.2 门槛

| 指标 | Q1 1 ms | Q2 500 us | Q3 250 us | Q4 混合压力 |
| --- | ---: | ---: | ---: | ---: |
| soak 时长 | >= 30 min | >= 30 min | >= 30 min | >= 30 min |
| 绝对 release jitter P50 | <= 2 us | <= 1 us | <= 1 us | <= 2 us |
| 绝对 release jitter P99 | <= 10 us | <= 5 us | <= 3 us | <= 11 us |
| 绝对 release jitter max | <= 50 us | <= 25 us | <= 15 us | <= 55 us |
| EtherCAT fast path P99 | <= 250 us | <= 125 us | <= 75 us | <= 275 us |
| RT CPU 利用率 | <= 50% | <= 60% | <= 70% | <= 55% |
| deadline miss | 0 | 0 | 0 | 0 |
| fault-free WKC mismatch | 0 | 0 | 0 | 0 |
| fault-free frame timeout | 0 | 0 | 0 | 0 |
| 激活后 heap allocation | 0 | 0 | 0 | 0 |
| 控制面导致的 P99 基线增幅 | 不适用 | 不适用 | 不适用 | <= 10% |

性能报告字段使用 `p99_regression_percent`。Q3 未通过只表示该硬件/拓扑不具备 stretch 等级，不能通过修改统计方法判定通过。

### 11.3 资源门槛

| 资源 | P0 目标 | 报告要求 |
| --- | --- | --- |
| 核心 RAM | 32 从站、1 KiB Domain 过程映像、8 帧槽、8 请求、256 trace records 时，核心状态目标 < 64 KiB | 分列 arena、ProcBuf、Domain 映像、DMA、stack、RTOS/SDK，不得只报总 RAM。 |
| Flash | CoE + DC、关闭可选协议的 Cortex-M `.text + .rodata` 目标 < 128 KiB | 记录编译器、flags、LTO、map file hash。 |
| RT stack | 用 watermark/MPU guard 测得最大使用量后保留 >= 30% 余量 | 故障、trace 开启和最大请求压力均覆盖。 |
| descriptor/buffer | 数量由 Frame Plan 和在途窗口决定，激活后固定 | 报告 TX/RX 数量、对齐、cache 属性和总字节。 |

## 12. Benchmark 与 HIL 设计

### 12.1 测试层次

1. `microbench`：wire encode/decode、O(1) match、copy span、cache 操作、ring push/pop。
2. `host simulation`：虚拟时钟、PCAP 回放、乱序/重复/短帧/旧 generation/WKC 异常。
3. `port loopback`：MAC/PHY 或硬件 loopback 下测 descriptor、cache 和 ISR 路径。
4. `HIL topology`：真实 8/16/32 轴或等效从站拓扑，DC、不同 PDO 和线缆长度。
5. `fault injection`：拔线、从站掉电、WKC 少计、延迟帧、RX ring 满、TX descriptor 饥饿、DC 跳变。
6. `mixed load`：SDO、诊断读取、其他 DMA、supervisor 重启、ROS/Zenoh 网络压力同时运行。
7. `soak`：至少 30 分钟；候选发布建议 8 小时，产品量产门槛由项目提高。

### 12.2 公平比较 IgH 的约束

若要比较 ESOP 与 IgH，必须保持：

- 同一主机/CPU 或明确不可同硬件时给出限制；
- 同一 NIC、PHY、线缆、从站顺序和供电状态；
- 同一周期、PDO mapping、Domain/DC 频率和控制面流量；
- 同一测试时长、样本定义、CPU 隔离和温度区间；
- IgH 分别报告 native driver 与 generic driver，不能混为一条基线；
- 同时报 P50/P99/max、错误计数、CPU、RAM 和线缆占用，不只比较均值。

结果表述只能是“在该测试配置下”，不能外推为所有平台绝对优劣。

### 12.3 必须生成的性能报告

每次资格测试输出 `performance_report.json`。最小结构：

```json
{
  "schema_version": "esop.performance.v1",
  "generated_at": "2026-09-03T00:00:00Z",
  "software": {
    "esop_commit": "<git-sha>",
    "config_hash": "<sha256>",
    "compiler": "<name-version>",
    "cflags": ["-O2"]
  },
  "platform": {
    "board": "<board>",
    "soc": "<soc>",
    "clock_hz": 0,
    "port": "<port-name>",
    "rtos_or_kernel": "<version>",
    "cache_policy": "<policy>"
  },
  "topology": {
    "slave_count": 0,
    "axis_count": 0,
    "pdo_bytes": 0,
    "frame_count": 0,
    "dc_enabled": true,
    "topology_manifest_hash": "<sha256>"
  },
  "run": {
    "period_ns": 500000,
    "duration_s": 1800,
    "cycles": 3600000,
    "temperature_c_min": 0,
    "temperature_c_max": 0
  },
  "latency_ns": {
    "release_jitter_abs": {"p50": 0, "p99": 0, "p999": 0, "max": 0},
    "fast_path": {"p50": 0, "p99": 0, "p999": 0, "max": 0},
    "rx_round_trip": {"p50": 0, "p99": 0, "p999": 0, "max": 0}
  },
  "errors": {
    "deadline_miss": 0,
    "wkc_mismatch": 0,
    "frame_timeout": 0,
    "rx_overflow": 0,
    "tx_starvation": 0,
    "unmatched": 0,
    "corrupt": 0
  },
  "copies": {
    "tx_bytes_per_cycle": 0,
    "rx_bytes_per_cycle": 0,
    "copy_spans_per_cycle": 0
  },
  "resources": {
    "core_arena_bytes": 0,
    "procbuf_bytes": 0,
    "dma_bytes": 0,
    "rt_stack_peak_bytes": 0,
    "text_rodata_bytes": 0,
    "rt_cpu_percent": 0
  },
  "qualification": {"scenario": "Q2", "passed": false, "failures": []}
}
```

报告生成器必须拒绝缺失 cycles、最大值、错误计数或配置 hash 的“通过”结果。原始 trace 可以抽样保存，但计数器和 max 不能抽样。

## 13. 验证需求

| ID | 优先级 | 需求 | 验收证据 |
| --- | --- | --- | --- |
| PERF-V001 | P0 | 激活后 cycle、service、request 和 diagnostic API 均不调用 heap。 | allocator wrapper + 30 分钟 Q4，调用次数为 0。 |
| PERF-V002 | P0 | 同一静态配置生成确定的 Frame Plan、schedule 和 hash。 | 多次构建/启动结果一致。 |
| PERF-V003 | P0 | RX index 匹配为 O(1)，未知/旧/重复 index 不修改 committed input。 | 单元测试、PCAP fuzz 和故障注入。 |
| PERF-V004 | P0 | 半帧、坏长度、type/size/address 不匹配及 WKC 异常均拒绝 commit。 | 每类错误的 State/Quality 断言。 |
| PERF-V005 | P0 | RX drain 同时受 frame、byte、time 上限约束。 | RX flood 下函数最大时长和 budget-exhausted 计数。 |
| PERF-V006 | P0 | P2/P3 流量不能改变 P0 Frame Plan 或造成 deadline miss。 | Q4 与空载对比，P99 回归 <= 10%。 |
| PERF-V007 | P0 | DMA 所有权和 cache 维护在所有声明支持的平台通过压力测试。 | 数据一致性、descriptor 状态和 cache fault 注入。 |
| PERF-V008 | P0 | 每 Domain 暴露 expected/actual WKC、last valid cycle、input age 和连续失败。 | HIL 拔线/掉站/恢复序列。 |
| PERF-V009 | P0 | 1 ms Q1 和 500 us Q2 达到第 11 节门槛。 | 完整 `performance_report.json`。 |
| PERF-V010 | P1 | 250 us Q3 对通过的平台形成明确支持标签。 | 独立报告；未通过平台不得宣称支持。 |
| PERF-V011 | P0 | 构建输出 RAM、Flash、stack、descriptor 和 copy budget。 | map 文件、watermark、build report。 |
| PERF-V012 | P0 | ROS 2、Zenoh、Protobuf、文件系统和文本日志不在 RT 核心依赖图。 | 链接 map、include graph 和符号黑名单。 |
| PERF-V013 | P0 | 64 位时间和 sequence 在 RV32/Cortex-M 上无撕裂读取。 | 并发 stress、TSAN host model、目标机长测。 |
| PERF-V014 | P1 | ESOP/IgH 比较遵守同硬件同负载原则并区分 IgH native/generic。 | benchmark manifest 和审查记录。 |
| PERF-V015 | P0 | 配置预算不足时激活失败并给出具体超限项。 | wire、index、arena、frame、schedule 五类超限测试。 |

## 14. 风险、退化与回退

| 风险 | 影响 | 缓解/回退 |
| --- | --- | --- |
| 预计算计划降低运行期灵活性 | 在线重映射或拓扑变化需重新激活 | P0 产品只支持静态拓扑；进入安全状态后 deactivate/reconfigure/activate。 |
| RX staging 增加 RAM 和一次提交拷贝 | 小 MCU 资源压力 | cfggen 合并 input span；可用双页指针切换，但不取消完整性校验。 |
| 256 项 index 表固定占用 RAM | 每端口约若干 KiB | 使用紧凑结构；其 O(1) 和确定性收益优先于链表节省。 |
| TX 一次 payload copy 成为瓶颈 | 高频大 PDO 周期 P99 增大 | 先优化 span 合并/cache 布局；有 benchmark 证据后增加可选 SG DMA。 |
| 轮询 budget 太小 | 延迟帧留到下一周期或超时 | 依据真实帧数设置；P0 descriptor 单独预留；预算不足激活失败。 |
| 轮询 budget 太大 | 故障流量占用 RT 时间 | time 上限强制退出；RX flood 记录并触发端口恢复。 |
| 多速率 hyperperiod 太大 | plan RAM 增长 | 限制周期集合；用相位/重复 pattern 压缩；超限时拒绝配置。 |
| Linux generic 路径抖动 | 开发测试误判产品性能 | 报告标记 `functional_only`；量产资格使用 MCU DMA 或单独合格端口。 |
| 控制面长期饥饿 | SDO/恢复无法完成 | 请求 deadline 和明确 `DEFERRED/TIMEOUT`；必要时进入维护模式暂停运动。 |
| 过度平台微优化 | core 分叉和维护成本 | 保留 Rust `no_std` 基线；平台优化在 port capability 下可关闭并做等价测试。 |

默认回退级别：

1. 250 us 不通过时回退 500 us；
2. 500 us 不通过时回退 1 ms；
3. 降低低速 Domain 频率或拆分帧/拓扑；
4. 停止并发维护请求；
5. 若 P0 仍不满足，降低该板卡支持等级，不在核心中添加不可验证的隐式特例。

## 15. 采用、改造与明确拒绝清单

### 15.1 直接采用的原则

- 调用者驱动、被动主站；
- 显式 receive/process/queue/send 生命周期；
- Domain 和 per-Domain WKC；
- 激活前配置、周期阶段稳定对象；
- 多数据报帧聚合；
- 轮询式 RX；
- 异步请求/FSM；
- DC 预分配；
- acquire/release 状态发布；
- 控制面失败时 try/defer 而非等待。

### 15.2 针对 MCU 改造的机制

- 内部分配改为调用方 arena；
- 运行期装箱改为预计算 Frame Plan；
- 链表队列改为连续 Slot 数组和固定 ring；
- RX 线性搜索改为 256 项索引表；
- Domain 直接 RX 覆盖改为 staging + validation + commit；
- 单一 byte budget 改为 wire byte/datagram/time 三重预算；
- 内核原生驱动轮询改为 MAC DMA descriptor 轮询；
- syslog/文本诊断改为固定二进制事件。

### 15.3 明确拒绝进入 ESOP core

- Linux kernel module、`net_device`、socket、`ioctl`、`mmap`、RTDM；
- 周期热路径链表遍历和按队列长度增长的匹配；
- 周期内分配、重新分配、阻塞 semaphore/mutex、sleep 或 busy wait；
- 未验证 RX 数据覆盖正式 ProcBuf；
- Protobuf/Zenoh/ROS 2/JSON/日志格式化进入 PDO 周期；
- 用 generic network stack 结果代表嵌入式 DMA 性能；
- 未经同条件 benchmark 的“性能优于 IgH”宣传结论。

## 16. 实施顺序

| 阶段 | 实现重点 | 性能出口条件 |
| --- | --- | --- |
| P0-A | arena、wire codec、descriptor port、固定 Slot、O(1) RX table | microbench、无分配、短帧/fuzz 通过 |
| P0-B | Domain、Frame Plan、single-rate PDO、WKC、staging commit | 8 轴 Q2 仿真和 HIL，无无效 commit |
| P0-C | DC、multi-rate schedule、控制面固定池 | Q1/Q2、Q4 隔离门槛通过 |
| P0-D | STM32/HPM 资格端口与完整报告 | 各支持板卡发布 capability matrix |
| P1 | Linux RT 资格端口、250 us stretch、可选 SG DMA 调研 | 只按独立报告开放能力标签 |

## 17. 独立实现与许可证边界

IgH EtherCAT Master 主要内核实现文件声明 GPLv2，部分公共/兼容头文件存在 LGPLv2.1 声明。ESOP 必须执行 clean-room 风格的独立实现：

- 本文只记录公开行为、算法思想、接口语义和性能观察；
- 禁止复制 IgH 源码、结构体布局、宏、注释表达、测试代码或生成文件；
- ESOP wire codec、状态机、Frame Plan、数据结构和测试向量独立设计；
- 规范常量和线缆字段以公开 EtherCAT/IEC/ETG 资料及合法授权为依据；
- 若未来链接、派生或复用任何第三方代码，必须单独完成许可证与分发义务审查；
- benchmark 可与 IgH 可执行系统做黑盒比较，但不构成源码复用许可。

## 18. 决策后果

正面结果：周期复杂度可由配置上限和计划长度直接解释；MCU 无需 Linux 基础设施；异常 RX 不会污染状态；控制面和 ROS/Zenoh 不会反向阻塞 PDO；平台性能可以用统一报告比较。

代价：激活期和 cfggen 更复杂；静态拓扑变化需要重新激活；staging 和索引表增加 RAM；首版保留一次 TX/RX 有界拷贝；每个目标板卡都必须完成 cache、DMA 和时钟资格验证。

这些代价与 ESOP 的产品定位一致：把复杂度前移到生成、激活和验证阶段，换取运行阶段的短路径、可预测性和可审计故障行为。

## 19. 参考源

- IgH EtherCAT Master，官方 GitLab 仓库，提交 `650888c587b1aa570c4d7211adaf39145d9e5ae3`，2026-08-21。
- IgH `README.md`：Realtime and Tuning，被动主站和调用者负责实时处理。
- IgH `FEATURES.md`：原生无中断驱动、Domain、多采样率和减少过程数据复制。
- IgH `include/ecrt.h`：activate、send/receive、Domain 外部内存、异步 SDO API 契约。
- IgH `master/master.c`：外部数据报环、发送预算、帧聚合、RX 匹配、超时、原子交接和 DC 数据报。
- IgH `master/domain.c`、`master/datagram_pair.c`：Domain 激活、逻辑映像、expected WKC、queue/process 和数据报预分配。
- IgH `master/device.c`、`devices/generic.c`：主动轮询与 generic socket 驱动路径。
- IgH `lib/master.c`、`lib/domain.c`：Linux 用户态 `ioctl`/`mmap` 过程映像接口。
