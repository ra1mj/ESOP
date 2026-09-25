# ESOP 软件产品需求文档（PRD）

- 产品：ESOP（EtherCAT Simple Operating System）
- 文档版本：1.1
- 日期：2026-09-03
- 状态：规划基线
- 最近实现状态更新：2026-09-25
- 面向版本：首个机器人控制产品线（R0-R4）
- 相关决策：[机器人软件规划](robotics-esop-software-plan.md)、[EtherCAT 主站需求](ethercat-master-requirements.md)、[实时与性能架构决策](esop-performance-architecture-decision.md)、[ETG/CiA 402 决策](esop-etg-cia402-master-requirements.md)、[运动生命周期守卫设计](esop-motion-lifecycle-guard.md)、[eBPF 运行时观测设计](esop-ebpf-runtime-observability.md)

## 1. 产品概述

ESOP 是面向机器人和机器控制设备的 EtherCAT 实时控制运行时。它为伺服驱动、分布式 IO 和受控外设提供确定性的周期过程数据交换、设备状态管理、诊断与对上层控制软件的稳定接口。

ESOP 不是通用操作系统，不替代 Linux、FreeRTOS、Zephyr、ROS 2 或运动规划系统。它的核心价值是：在 MCU、RTOS 或受控 Linux 实时环境中，将 EtherCAT 周期通信和驱动状态安全地交付给机器人控制系统，同时避免网络、序列化、文件系统和高层框架进入硬实时路径。

首个产品闭环为：一个 Linux 监督节点与一个 STM32 或 HPMicro 实时节点协作，控制 2 个 CiA 402 EtherCAT 伺服和 1 个 EtherCAT DI/DO 模块，并向 Zenoh 和 ROS 2 `ros2_control` 提供可验证的状态、命令和诊断接口。

## 2. 背景与问题

机器人产品需要将多轴伺服、IO、编码器和状态诊断以固定周期接入控制器。现有通用 EtherCAT 主站通常假定 Linux 内核、动态内存、平台特定网卡驱动或复杂运维接口，难以直接满足 MCU 端的依赖、内存和可证明时延约束。另一方面，ROS 2、Zenoh 和 Protobuf 适合上层控制、远程连接与观测，但其调度和序列化行为不能作为 PDO 周期的一部分。

ESOP 要解决以下产品问题：

1. 在 STM32、HPMicro、通用 ARM 和 Linux 开发环境中提供同一套 EtherCAT 核心能力。
2. 让机器人应用以稳定的关节、IO、时钟质量和故障语义使用现场总线，而不是暴露原始 PDO 字节流。
3. 在 WKC 异常、驱动故障、时钟失锁、命令过期和监督节点掉线时，给出确定、可审计且不夸大安全等级的降级行为。
4. 将实时控制面与 ROS 2、Zenoh、Protobuf、记录和远程维护解耦，保证任一非实时服务失效不阻塞 EtherCAT 周期。
5. 为每一种支持的板卡、拓扑、驱动、模式和周期生成可复核的构建、性能与硬件在环证据。

## 3. 产品目标与成功标准

### 3.1 产品目标

1. 提供基于 Rust `no_std` 的可移植 EtherCAT 主站核心，支持裸机、RTOS 和 Linux 用户态端口；平台端口可按需使用 Rust `std`。
2. 支持机器人首发所需的在线扫描、静态配置、PDO、CoE SDO、Distributed Clocks（DC）、CiA 402 和分布式 IO。
3. 以 ProcBuf 作为实时状态/命令 ABI，以版本化 Protobuf 作为非实时外部数据契约。
4. 以 `ros2_control` 硬件接口和 Zenoh 网关接入机器人软件生态，而不实现新的 ROS 2 RMW。
5. 在声明支持的平台与拓扑上，以 1 ms 和 500 us 周期实现零 deadline miss、零无故障 WKC mismatch、零无故障 frame timeout 的资格测试。
6. 以运动生命周期守卫（MLG）持续检测运动前提，并在不满足时阻止使能、执行受配置约束的停止动作和锁存故障。
7. 在 Linux 监督域以 eBPF 采集运行时证据，回答“哪个进程、线程、CPU、IRQ、网络或内存事件导致了这次周期异常”。

### 3.2 成功标准

| 维度 | R4 发布成功标准 |
| --- | --- |
| 互操作 | 至少两种厂商的 CiA 402 驱动和至少一种 EtherCAT IO 模块完成 HIL 启动、运行、异常和恢复验证。 |
| 实时性 | 在合格的 Q1（1 ms）和 Q2（500 us）场景中，连续运行至少 30 分钟，无 deadline miss、无无故障 WKC mismatch、无无故障 frame timeout。 |
| 可移植性 | Linux raw、一个 STM32 和一个 HPMicro 端口执行同一核心基础用例；平台差异不进入协议核心。 |
| 数据完整性 | 半帧、迟到帧、重复帧、旧帧、长度不符帧和 WKC 不合格帧均不得污染已提交的机器人状态。 |
| 生命周期防护 | 未通过平台、配置、总线、时钟、驱动、命令许可与外部安全链观测门槛时，系统无法进入或保持 `MOTION_ACTIVE`。 |
| 运行时归因 | 发生周期超时、网关断连、CPU 抖动、网络丢包或进程退出时，能够关联到具体时间窗口、组件和内核/用户态证据。 |
| 集成 | `ros2_control` 可驱动双轴轨迹示例；Zenoh 可发布状态和事件，并按授权、TTL 与序号处理命令。 |
| 可审计性 | 每个候选版本提供能力清单、构建资源报告、性能报告、HIL 拓扑清单和已知限制。 |

## 4. 用户与使用场景

| 用户/角色 | 主要目标 | 使用 ESOP 的方式 |
| --- | --- | --- |
| 实时固件工程师 | 在受限硬件上稳定驱动 EtherCAT 网络 | 配置主站、端口、周期和设备 profile，分析实时诊断。 |
| 机器人集成工程师 | 将关节和 IO 接入机器人控制栈 | 使用生成的设备配置、ProcBuf、ROS 2 `ros2_control` 配置和兼容矩阵。 |
| 控制算法工程师 | 以可靠的状态与命令语义控制机器人 | 读写带时间戳、质量位和时效约束的关节/IO 数据。 |
| 系统运维/测试工程师 | 定位现场总线和驱动异常 | 查询拓扑、AL 状态、WKC、DC、CiA 402 和事件记录，执行受控维护请求。 |
| 产品/质量团队 | 对客户做准确能力声明 | 审核平台、驱动、周期、协议与测试证据，避免未验证的兼容性或认证承诺。 |

## 5. 产品边界

### 5.1 首发范围（P0）

1. 单一 EtherCAT 网段、一个活动主站端口、线型或树型从站拓扑。
2. 可配置静态上限，基线支持至少 32 个从站；不以扫描结果触发堆分配。
3. EtherCAT 常用物理与逻辑数据报、在线扫描、SII 基础读取、ESC 寄存器访问和 EtherCAT State Machine（INIT、PREOP、SAFEOP、OP）。
4. 静态 Sync Manager、FMMU、PDO 映射、Domain、WKC、过程数据周期交换和分布式时钟。
5. CoE 邮箱、SDO expedited/segmented upload/download、PDO 配置、Mailbox Resilient Layer 和 CoE Emergency。
6. CiA 402 PDS 状态机、CSP、CSV、CST、驱动状态与命令时效联合控制。
7. ProcBuf、运动生命周期守卫（MLG）、设备/profile 插件、固定事件记录、结构化诊断与故障降级。
8. Linux raw 开发/HIL 端口、一个 STM32 端口和一个 HPMicro 端口。
9. 配置生成、资源/线缆预算报告、PCAP 回放、仿真和 HIL 测试。

### 5.2 后续范围（P1）

1. 多 Domain 多速率调度、显式设备识别、Complete Access 和 SDO Information。
2. 机器人外设适配（CAN-FD、I2C、SPI、UART、USB、GPIO）与统一设备生命周期。
3. 版本化 Protobuf、Zenoh 网关、访问控制、命令审计、记录与回放。
4. ROS 2 `ros2_control` `SystemInterface`、ROS 2 bridge 和已验证的部署健康检查。
5. Homing、Profile Position/Velocity/Torque 及厂商能力/quirk 描述。
6. FreeRTOS、Zephyr 和性能资格 Linux 端口。
7. Linux 监督域的 eBPF 运行时观测器、问题归因、事件窗口采集与观测健康检查。

### 5.3 产品专项范围（P2）

1. FoE、EoE、SoE、AoE、VoE、BOOT、冗余端口和多主站。
2. FSoE、安全 PLC 集成和安全认证相关交付物。
3. 力控、复杂末端工具、高带宽传感器和车队/WAN 产品能力。
4. ETG 官方一致性测试、正式认证或商标使用流程。

### 5.4 明确不在范围内

1. Linux 内核模块、私有网卡驱动 fork、字符设备、ioctl、RTDM、systemd 服务或通用命令行运维工具。
2. 在 MCU 固件中运行 ROS 2、Zenoh、Protobuf runtime、JSON、文件系统、DNS 或远程网络服务。
3. 在线解析大型 ENI/XML；该类输入只能由宿主配置工具转换为静态配置。
4. 运动学、轨迹规划、碰撞规划、视觉、SLAM、行为树和机器学习推理。
5. 将普通 CoE/PDO、ROS 2 QoS 或普通网络命令表述为功能安全通道。
6. 未经适用外部流程验证即宣称 EtherCAT 一致性、认证或对全部设备兼容。

## 6. 产品原则与约束

1. 周期路径优先于控制面：PDO/DC 与运动相关数据为 P0；扫描、SDO、维护和诊断只能使用明确剩余预算。
2. 激活后冻结：运行期容量、映射、计划和内存由已激活配置决定；资源不足必须在生成或激活阶段失败。
3. 数据有效性优先于新鲜度：不完整或不可信的新输入不能与旧输入混合发布为有效状态。
4. 一个实时所有者：同一实时运行时只有一个周期调用上下文；非实时上下文仅通过快照、固定请求队列或只读状态交互。
5. 事实驱动的能力声明：仅对已通过指定板卡、拓扑、驱动、模式和周期测试的组合声明支持。
6. 依赖隔离：协议核心不得依赖平台 SDK、RTOS、POSIX、C++ runtime、ROS 2、Zenoh、Protobuf 或动态分配器。
7. 运动 fail-closed：运动只能由完整、当前且可追溯的准入证据开启或维持；任何未分类、未知或过期状态均视为不满足运动前提。
8. 观测不反向阻塞：eBPF、观测器、事件传输和问题分析只能旁路采集；不得进入 MCU/RT EtherCAT 周期的必要执行链。

## 7. 功能需求

优先级说明：P0 为首发阻塞项；P1 为首发后产品化能力；P2 为按产品需求交付的可选能力。

### 7.1 生命周期、发现与配置

| ID | 优先级 | 需求 | 验收标准 |
| --- | --- | --- | --- |
| FR-001 | P0 | 系统应执行在线扫描，发现从站、读取基础 SII/ESC 信息、分配固定站地址，并生成拓扑快照。 | 1、8、32 从站 HIL 中地址唯一，拓扑、vendor ID、product code、revision 和 serial 可查询。 |
| FR-002 | P0 | 系统应管理 INIT、PREOP、SAFEOP、OP 状态转换、超时、错误确认和 AL status code。 | 每条状态转换、超时和 AL 错误有自动化或 HIL 故障注入证据，事件含从站、请求状态、实际状态和错误码。 |
| FR-003 | P0 | 激活前应将实际网络与静态配置的 alias/position、vendor/product/revision 进行比对；不匹配时不得进入 OP。 | 正常、位置错误、型号错误和 revision 错误测试均得到预期拒绝结果。 |
| FR-004 | P0 | 系统应支持静态 SM、FMMU、watchdog、PDO assignment/mapping 和固定逻辑地址配置。 | 同一配置重复激活的映射、预期 WKC 和帧计划一致；配置期 PDO 写入可 read-back 验证。 |
| FR-005 | P1 | 系统应支持显式设备识别与完整 SII PDO/SM 信息校验，用于防止同型号设备错位或换线。 | 交换同型号设备或变更识别对象后，按配置拒绝激活并给出原因。 |
| FR-006 | P0 | 配置生成工具应把设备/ESI/产品配置转为静态固件配置和可读报告，不将 XML 运行时带入固件。 | 生成 C 配置、ProcBuf 布局、设备清单和构建报告；同一输入生成一致的配置 hash。 |

### 7.2 EtherCAT 周期数据与 Domain

| ID | 优先级 | 需求 | 验收标准 |
| --- | --- | --- | --- |
| FR-007 | P0 | 系统应支持 APRD/APWR/APRW、FPRD/FPWR/FPRW、BRD/BWR/BRW、LRD/LWR/LRW、ARMW、FRMW 的正确编码、解析和 WKC 提取。 | 每种命令具有 golden frame、长度边界、follow 位和异常解析测试。 |
| FR-008 | P0 | 系统应支持一个 Ethernet 帧中的多个 EtherCAT 数据报，并在激活阶段为每周期生成固定发送计划。 | 单数据报、多数据报和超过单帧上限场景均验证帧长、顺序、WKC 和截止时间。 |
| FR-009 | P0 | 每个 Domain 应提供稳定的 RxPDO/TxPDO 位偏移、逻辑地址、预期/实际 WKC、有效性、最后成功周期和输入年龄。 | 位域、8/16/32/64 位、signed/unsigned、多 Domain 和不同更新率测试通过。 |
| FR-010 | P0 | 周期接收仅在帧、数据报、地址、长度、世代和 WKC 均符合计划时提交输入。 | 丢帧、半帧、重复帧、旧帧、WKC 失配时，已提交输入保持上一有效值并更新质量状态。 |
| FR-011 | P0 | 周期收发与处理必须受固定时间、字节和帧数预算限制，不得无限轮询、忙等或等待控制面。 | RX flood 和未回帧场景下，周期函数最大时长不超过预算，且产生 budget-exhausted 诊断。 |
| FR-012 | P0 | 系统应支持静态配置的从站到从站经主站数据复制，并携带源数据质量。 | 源从站到目标从站路径不超过两周期；源 WKC 异常或数据过期时目标质量正确降级。 |
| FR-013 | P1 | 系统应支持多 Domain 不同周期和相位，但所有周期必须是基准 tick 的整数倍。 | 生成 hyperperiod 调度计划；超出计划大小、帧、线缆或 index 预算的配置在激活时失败。 |

### 7.3 邮箱、CoE 与 Distributed Clocks

| ID | 优先级 | 需求 | 验收标准 |
| --- | --- | --- | --- |
| FR-014 | P0 | 邮箱收发、超时、计数器、重复帧和错误响应应通过异步请求状态机执行。 | 请求跨多个服务周期完成，周期数据面无等待；丢帧、重复、计数器回绕和超时测试通过。 |
| FR-015 | P0 | 系统应支持 CoE SDO expedited 与 segmented upload/download，并返回 abort code 和请求上下文。 | 标准对象读写、分段读写、abort、超时与缓冲不足测试通过。 |
| FR-016 | P0 | 系统应实现 Mailbox Resilient Layer、输入邮箱轮询/Status Bit 和 CoE Emergency 接收。 | PollTime、Status Bit、多驱动 Emergency、事件环满和邮箱恢复测试通过。 |
| FR-017 | P1 | 系统应按设备能力支持 Complete Access 和 SDO Information，且允许在产品配置中禁用。 | 支持与拒绝 Complete Access 的设备均可正确配置；对象能力与 ESI/对象字典交叉验证。 |
| FR-018 | P0 | 系统应识别 DC 能力、选择参考时钟、配置应用时间、SYNC0 周期/相位并监测时钟质量。 | DC 与非 DC 拓扑启动均通过；记录 offset、jitter、last sync、失锁次数和同步窗口状态。 |
| FR-019 | P0 | 当 DC 未锁定、WKC 无效、命令过期或驱动状态异常时，系统不得发布新的有效运动目标。 | DC 失锁、WKC 异常、命令超时、驱动 fault 联合故障矩阵通过。 |

### 7.4 CiA 402 与设备模型

| ID | 优先级 | 需求 | 验收标准 |
| --- | --- | --- | --- |
| FR-020 | P0 | 系统应以独立 profile 实现 CiA 402 PDS 状态机，并根据 Statusword 生成合法 Controlword 转换。 | 全状态、全转换、非法状态、超时和 fault reset 单元测试；至少两种厂商驱动 HIL。 |
| FR-021 | P0 | 系统应支持 `0x6040`、`0x6041`、`0x6060`、`0x6061`、`0x603F` 及所选模式的目标/实际值对象。 | 对象缺失、只读、写入拒绝与 read-back 测试；设备能力、访问权和缩放可查询。 |
| FR-022 | P0 | 机器人基线应支持 CSP、CSV 和 CST；模式切换必须经安全序列并等待实际模式确认。 | 三模式启动、停止、切换、拒绝切换和超时 HIL；首周期无位置跳变、速度突变或转矩阶跃。 |
| FR-023 | P1 | 系统应通过 capability/quirk 描述扩展 homing、profile 模式和厂商对象，不污染 EtherCAT 核心。 | 两厂商驱动生成不同 profile 配置，但共享相同主站核心和公共状态机。 |
| FR-024 | P0 | 设备/profile 生命周期应统一覆盖 probe、identify、configure、verify、activate、cyclic read/write、degraded/fault、recover 和 deactivate。 | EtherCAT 驱动、IO 与至少一种非 EtherCAT 外设均可按统一生命周期报告状态。 |
| FR-025 | P0 | 外设的非周期事务不得阻塞 EtherCAT PDO 周期，且其时钟精度不得被误表述为 EtherCAT DC 精度。 | 在 CAN-FD 或 I2C 故障压力下，P0 周期的 P99 指标仍满足资格门槛。 |

### 7.5 ProcBuf、IPC 与外部接口

| ID | 优先级 | 需求 | 验收标准 |
| --- | --- | --- | --- |
| FR-026 | P0 | ProcBuf 应提供固定布局的 Command、State、Quality 和 Event 数据区，包含 ABI 版本、布局 hash、robot ID、boot ID、序号和时间戳。 | 生成布局与实际 PDO/设备映射一致；ABI/hash/boot ID 不匹配时拒绝数据交互。 |
| FR-027 | P0 | ProcBuf 的命令与状态应保证单写者/单读者一致性；读者不得看到部分状态，过期命令不得进入 PDO。 | 并发、重启、cache、序号、时效和掉线注入测试通过。 |
| FR-028 | P0 | 每个关节应暴露请求模式、目标位置/速度/转矩、实际值、使能、状态字、故障、时间戳和质量。 | 生成 layout、实物 PDO offset 与 SI 单位/缩放定义交叉验证。 |
| FR-029 | P1 | IPC 应支持共享内存、RPMsg 或 Unix domain socket 的受控实现，并传递版本、序号、时间戳、质量与掉线状态。 | 每种声明支持的 IPC 完成 supervisor 重启、boot ID 变化、延迟和断线测试。 |
| FR-030 | P1 | 外部 API 应以 `proto/esop/v1/` 下的版本化 Protobuf 定义配置、状态、事件、维护与诊断数据。 | 新旧 reader/writer 兼容性组合进入 CI；删除字段均 reserved，字段号不复用。 |
| FR-031 | P1 | Zenoh 网关应在 Linux 监督域发布状态/事件/诊断，接收受控命令和查询。 | key namespace、来源身份、ACL、TTL、序号、重放、限流、断连与恢复测试通过。 |
| FR-032 | P1 | `ros2_control` 硬件接口的 `read()`/`write()` 仅访问 ProcBuf/IPC，不直接访问 EtherCAT 端口或网络。 | 依赖检查、单元测试、双轴 `joint_trajectory_controller` 仿真和 HIL 演示通过。 |
| FR-033 | P1 | ROS 2 bridge 应显式映射 ROS 类型与 ESOP Protobuf；不得实施隐式泛型双向转换。 | topic/service/action 映射具有契约测试；ROS 2 类型仍保持 `rosidl`/CDR 语义。 |

### 7.6 诊断、维护与可交付证据

| ID | 优先级 | 需求 | 验收标准 |
| --- | --- | --- | --- |
| FR-034 | P0 | 系统应提供固定容量事件记录，至少包含时间、严重级别、模块、从站/设备、帧/请求 ID、错误码和上下文。 | 事件环溢出可计数且不破坏周期；五类核心故障均形成结构化事件。 |
| FR-035 | P0 | 系统应提供 master、link、slave、Domain、DC、CiA 402、命令年龄和 ProcBuf 状态快照。 | 查询不分配内存、不格式化文本；并发压力下快照一致。 |
| FR-036 | P0 | 系统应在命令过期、WKC 连续异常、驱动离开 OP/fault、DC 异常、链路断开和 supervisor 重启时执行显式降级策略。 | 每类故障的 hold、ramp-to-zero、quick stop 或 disable 决策均可配置、可观测、可 HIL 验证。 |
| FR-037 | P1 | 每个构建应生成 `robot_build_report.json`，包含设备清单、静态内存、ProcBuf、PDO、帧、线缆、WKC、copy 与周期预算。 | CI 审核报告，且配置/资源超限时输出明确的失败项。 |
| FR-038 | P0 | 每次性能资格测试应生成 `performance_report.json`，记录配置、平台、拓扑、周期、jitter、fast path、错误、资源和结论。 | 缺少 cycles、最大值、错误计数或配置 hash 的报告不得判定通过。 |

### 7.7 运动生命周期安全检测

运动生命周期守卫（MLG）是普通控制域的 fail-closed 运动许可机制，用于防止未完成资格检查、状态失效或未经恢复的系统进入/保持运动。它不是功能安全组件，不替代 STO、FSoE、安全 PLC、机械防护或认证风险评估。

| ID | 优先级 | 需求 | 验收标准 |
| --- | --- | --- | --- |
| FR-039 | P0 | MLG 应在启动和每个实时周期采集并记录平台、配置、拓扑、AL/Domain/WKC、DC、驱动、命令、监督、执行预算和外部安全链观测状态。 | 每个门槛有当前值、时间戳、连续好/坏周期计数、故障码和最近状态转换记录。 |
| FR-040 | P0 | 仅当所有必需门槛已在配置的稳定窗口内有效，且存在当前 boot ID 的有效运动许可时，MLG 才可进入或保持 `MOTION_ACTIVE`。 | 状态机模型和 HIL 表明任何单一必需门槛为 false/unknown/stale 时均不能进入或继续运动。 |
| FR-041 | P0 | MLG 应对每个门槛使用显式的进入阈值、退出阈值、连续周期数和时效；不得以总分或平均健康分掩盖单点失效。 | 边界抖动、短暂恢复、时钟跳变和序号重放测试中，状态转换符合配置并可解释。 |
| FR-042 | P0 | 运行期间门槛失效时，MLG 应按故障类别请求每轴已配置的 hold、ramp-to-zero、quick stop 或 disable，并阻止新运动目标；不可恢复故障必须锁存。 | 命令过期、WKC、DC、驱动、链路、deadline、外部 safety inhibit 与 supervisor 重启的 HIL 故障矩阵通过。 |
| FR-043 | P0 | MLG 的恢复必须包含原因消失、状态稳定、明确的恢复请求和新的 motion permit；不得因通信自动恢复或持续置位的 fault reset 自动重新使能。 | 故障注入后仅重连/仅清 fault/仅写 enable 均无法恢复；完整恢复流程后才能重新使能。 |
| FR-044 | P1 | 外部命令进入实时域前应被转为固定大小的 motion permit，至少携带 boot ID、来源、许可 epoch、轴掩码、序号、到期时间和策略版本。 | 未授权、旧 boot ID、重复序号、越权轴、过期 permit 与策略版本不匹配均被拒绝并审计。 |
| FR-045 | P0 | MLG 应将 lifecycle state、门槛位图、首个阻塞原因、停止动作、故障锁存原因和恢复计数发布到 ProcBuf 与事件记录。 | supervisor、ROS 2 和 Zenoh 能读取同一状态语义；事件与 ProcBuf 序号、时间戳和 boot ID 可关联。 |
| FR-046 | P0 | 维护模式、重配置和固件升级期间，MLG 应撤销运动许可并阻止 CiA 402 `Operation enabled`；离开维护模式必须重新执行完整资格流程。 | 维护请求、配置 hash 改变、拓扑变化和升级模拟下，驱动无法保持运动使能。 |

### 7.8 Linux 运行时观测与问题归因

eBPF 观测器只部署在 Linux 监督域或 Linux 实时端口；STM32/HPMicro 实时节点使用自身固定事件、计数器和 ProcBuf 诊断。eBPF 观测不可用时，ESOP 仍必须依靠 RT 域自身的 MLG、WKC、DC、deadline 和驱动证据运行，不得把 eBPF 当作唯一安全门槛。

| ID | 优先级 | 需求 | 验收标准 |
| --- | --- | --- | --- |
| FR-047 | P1 | 系统应提供 eBPF agent 生命周期：内核能力探测、程序加载/验证/挂载、map/ringbuf 初始化、版本报告、健康心跳和安全卸载。 | 在支持、缺少 BTF、权限不足、程序 verifier 拒绝、ringbuf 满和 agent 重启场景下均有明确状态与降级行为。 |
| FR-048 | P1 | eBPF 应观测调度、IRQ/softirq、网络收发/丢弃、页错误、OOM/进程退出、CPU 迁移/限频和 ESOP/ROS/Zenoh 用户态关键函数。 | 至少能识别 scheduler stall、IRQ storm、NIC drop、page fault、CPU throttle、process crash、gateway stall 和 raw-port syscall stall；当前代码已具备有界调度唤醒延迟证据、按受跟踪 TID 聚合且只在迁移计数达到阈值并关联 transport-risk cycle 时生成的低置信度 `HOST_SCHEDULER_STALL` 证据、有界硬 IRQ/softirq 时长证据、按 EtherType/ifindex 聚合并关联风险周期的 `HOST_NIC_DROP` 证据、按 CPU/进程窗口聚合且关联风险周期的 `HOST_PAGE_FAULT` 证据、按 cpufreq policy CPU 去重且只在 `max_freq` 低于产品配置下限并关联风险周期时生成的 `HOST_CPU_THROTTLE` 证据，只把受跟踪进程主线程退出升级为 `USER_COMPONENT_EXIT`、按 `oom:mark_victim` 受害 PID 生成 `HOST_OOM` 的硬事实链路，以及通过稳定 v1 marker、独立固定 1024 项 LRU 状态和可选原子 uprobe 对测量 Zenoh gateway publish/callback 的 `GATEWAY_STALL` 与 Linux raw-port `send(2)`/非阻塞 `recv(2)` 系统调用边界的 `HOST_PORT_STALL` 路径。两类时长证据都严格要求 `duration_ns > threshold`、字段自洽并关联 transport-risk cycle；raw-port 证据不代表驱动队列、NIC DMA、线缆、从站或完整周期根因。gateway、raw-port、process-exit、OOM、scheduler-migration、scheduler-runqueue、softirq、page-fault 与 network-drop 子路径均已增加特权托管 Linux 资格：前两者覆盖真实 CO-RE verifier/load、四个 gateway marker 或 raw-port marker pair uprobe、25 ms direct-marker/Unix `recv(2)` 延迟、ringbuf、统计和对应相关器；process-exit 覆盖同步 worker 抑制与 leader 退出；OOM 覆盖独立 cgroup v2 单进程、32 MiB `memory.max`、有界匿名页压力、局部 `oom_kill=1`、`SIGKILL`、唯一 victim-PID `HostOom` 和 Failed heartbeat；scheduler migration 覆盖精确 worker TID、实际 allowed CPU 上 A→B→A 两次受控移动、首次低于阈值抑制、唯一 `CpuMigration`/Warning `HostSchedulerStall` 和 Degraded heartbeat；scheduler runqueue 覆盖 CPU A 私有 futex waiter、CPU B 控制线程、CPU A 有界 `SCHED_FIFO` blocker、唯一唤醒到切换时长、Error `HostSchedulerStall`/`ControlledStop` 和 Degraded heartbeat；softirq 覆盖 CPU/vector 过滤下的受控 loopback `NET_RX` 时长；page fault 覆盖预热独立子进程、16 个匿名页首次写入、精确 `ru_minflt`/BPF/evidence 计数和唯一 Warning `HostPageFault`；network drop 覆盖唯一 veth、协议/方向负向控制、四个未处理 EtherCAT 帧、独立 `rx_dropped` delta、唯一 Error/confidence-75 `HostNicDrop` 和 Degraded heartbeat。gateway 证据不代表 live Zenoh Session、router/transport queue、IPC、序列化、permit 或 reconnect 根因，raw-port 证据不代表真实 AF_PACKET/NIC 负载，network-drop 证据不代表物理 NIC、driver/NAPI/XDP/qdisc、队列压力、拥塞或真实 EtherCAT 设备，OOM 证据不代表受害 TGID、namespace/cgroup identity、触发者、全局压力、容器委派或重启策略，迁移证据不代表迁移原因、亲和性错误、实际停顿时长或 cache/NUMA 影响，runqueue 证据不代表自然负载根因、产品优先级或 CPU 隔离策略；这些资格均不代表生产目标内核。ROS 2、recorder、raw-port cycle/driver/NIC 边界、Zenoh IPC/序列化/permit/reconnect 等其余用户态关键函数，以及生产内核、自然负载下的 runqueue 根因、硬 IRQ/真实 NIC/持续 softirq 压力、物理 NIC/driver/NAPI/qdisc/队列压力下的丢包、页错误内存压力/major-minor 归因、全局 OOM/受害 TGID 与 cgroup 归因、限频、开销/WCET 和长时 HIL 资格仍需单独完成。 |
| FR-049 | P1 | 观测事件应与 ESOP `boot_id`、cycle sequence、组件 PID/TID、CPU、网卡、ProcBuf transition sequence 和 monotonic time 关联。 | 一次周期异常可以从 `performance_report` 追溯到对应的 eBPF 事件窗口和组件。 |
| FR-050 | P1 | agent 应在内核侧优先聚合计数/直方图，仅在触发阈值或诊断窗口内发送固定大小事件；事件传输不得阻塞被观测进程。 | 高频调度、网络和页错误压力下 ringbuf 丢失计数可见；调度迁移、网络与页错误窗口只在首次达到阈值时发送固定事件，迁移策略 epoch/窗口到期可重置计数，cpufreq policy 在同一低频 episode 只发送一次并在恢复后重置，agent 不等待、不向 RT 线程注入锁或同步调用。 |
| FR-051 | P1 | 系统应生成结构化 `RuntimeIncident`，包含 incident ID、级别、原因码、时间窗口、证据、关联周期、影响组件、丢失计数和建议动作。 | 运维界面/Zenoh/Protobuf 能按 incident ID 聚合同一问题的多条证据，而不是只显示孤立日志。 |
| FR-052 | P1 | eBPF 观测只能向 MLG 提供 `HOST_OBSERVATION` 证据或监督心跳，不能直接修改 CiA 402 controlword、绕过 MLG 或成为认证安全通道。 | agent 停止、事件误报、恶意事件和观测延迟测试中，运动许可仍只由 MLG 与产品安全策略裁决。 |

FR-048 的 process-crash 子路径现已增加特权托管 Linux 资格：
`make test-ebpf-process-exit-runtime` 只加载并要求 `sched_process_exit`，在把
`tracked_pid` 切换到同步子进程后，先证明一个已 join 的 worker 退出只增加
`thread_exits_ignored` 且 observer 保持 Healthy，再证明 leader 正常退出生成
唯一 `KernelProcess/ProcessExit`、Critical `UserComponentExit`/`LatchFault` 和
Failed heartbeat；严格报告写入
`build/ebpf_process_exit_qualification.json`。该结果只覆盖托管内核上的共享
leader-exit-to-incident/health 链，不含 exit code/signal，不证明线程组完全死亡，
也不覆盖 OOM、PID namespace/cgroup、组件重启、生产内核、开销/WCET 或长时
HIL，因此 FR-048 仍保持部分实现。

FR-048 的 OOM 子路径现已增加独立特权托管 Linux 资格：
`make test-ebpf-oom-runtime` 要求 cgroup v2 根层 memory controller，在新建叶子中
只放入一个已预热的单线程子进程，设置 32 MiB `memory.max`、关闭可用 swap 和
group OOM，再只加载并要求 `oom:mark_victim`。子进程对有界 128 MiB 匿名映射
逐页写入后必须由 `SIGKILL` 结束；`memory.events.local` 必须记录至少一次 OOM、
恰好一次 OOM kill 和零 group kill，同时 BPF 必须只生成唯一
`KernelMemory/OomKill`、Critical/confidence-100 `HostOom`/`LatchFault`、零 loss 和
fault `0x45422002` 的 Failed heartbeat。严格报告写入
`build/ebpf_oom_qualification.json`，且仅在子进程已回收、cgroup 为空并删除成功后
原子发布。该结果只覆盖 hosted cgroup-v2 单进程 memcg victim-PID/incident/health
链，不证明 victim TGID、PID namespace/cgroup identity、触发任务、全局内存压力、
容器委派、组件重启、生产内核、开销/WCET 或长时 HIL，因此 FR-048 仍保持部分实现。

FR-048 的 scheduler-migration 子路径现已增加独立特权托管 Linux 资格：
`make test-ebpf-scheduler-migration-runtime` 只加载并要求
`sched_migrate_task`，从实际 allowed affinity 选取两个 CPU，把同步且持续可运行的
worker 固定到 CPU A 后启用精确 TID 跟踪，再强制 A→B→A。第一次迁移必须只增加
内核计数，第二次才生成唯一 `KernelScheduler/CpuMigration`、Warning/confidence-60
`HostSchedulerStall`/`DegradeHostObservation` 和 fault `0x45422001` 的 Degraded
heartbeat；严格报告写入 `build/ebpf_scheduler_migration_qualification.json`。
该结果只覆盖 hosted-kernel migration-to-incident/health 共享链，不证明迁移
原因、亲和性策略错误、调度停顿时长、cache/NUMA 影响、生产内核、开销/WCET
或长时 HIL，因此 FR-048 仍保持部分实现。

FR-048 的 scheduler-runqueue 子路径现已增加独立特权托管 Linux 资格：
`make test-ebpf-scheduler-runqueue-runtime` 只加载并要求 `sched_wakeup` 与
`sched_switch`，从实际 allowed affinity 选取两个 CPU，将精确 target TID 固定在
CPU A 并确认其进入私有 futex 睡眠；控制线程保留在 CPU B，CPU A 的有界
`SCHED_FIFO` blocker 激活后唤醒唯一 waiter。target 在 25 ms hold 内不得完成，
blocker 释放后必须生成唯一 `KernelScheduler/SchedulerRunqueueLatency`、
Error/confidence-70 `HostSchedulerStall`/`ControlledStop` 和 fault `0x45422001` 的
Degraded heartbeat；严格报告写入
`build/ebpf_scheduler_runqueue_qualification.json`。该结果只覆盖 hosted-kernel
controlled wake-to-switch/incident/health 共享链，不证明自然负载根因、普通调度
行为、产品优先级/CPU 隔离、生产内核、开销/WCET 或长时 HIL，因此 FR-048
仍保持部分实现。

FR-048 的 softirq-duration 子路径现已增加独立特权托管 Linux 资格：
`make test-ebpf-softirq-runtime` 从实际 allowed affinity 选择并固定一个安静 CPU，
只加载并要求 `softirq_entry`/`softirq_exit`，在不改变 176 字节 context ABI 的前提下
精确过滤该 CPU 与 Linux `NET_RX` vector 3。夹具使用一次
`UDP_SEGMENT=1200` 的 64,800 字节 loopback 发送并完整接收 54 个 datagram；独立
校准运行测得 handler 时长后，正式运行使用其八分之一作为阈值，并要求唯一
`KernelIrq/SoftirqCpuTime`、Error/confidence-70 `HostIrqStorm`/`ControlledStop`、
零 loss 和 fault `0x45422001` 的 Degraded heartbeat。严格报告写入
`build/ebpf_softirq_qualification.json`。该结果把“受控 hosted loopback `NET_RX`
softirq 注入”从开放项中移除，但不证明硬 IRQ、真实 NIC/driver/NAPI、产品中断
预算、持续 softirq 压力、任务因果归属、生产内核、开销/WCET 或长时 HIL，
因此 FR-048 仍保持部分实现。

FR-048 的 page-fault 子路径现已增加独立特权托管 x86_64 Linux 资格：
`make test-ebpf-page-fault-runtime` 在加载 BPF 前启动、固定并预热独立子进程，
只加载并要求 `exceptions:page_fault_user`，再对子进程 16 个带 guard page 的未驻留
匿名页各执行一次首次写入。夹具要求子进程 `ru_minflt` 增量、BPF `page_faults`
与 evidence count 均精确为 16，只生成唯一 `KernelMemory/PageFault`、
Warning/confidence-65 `HostPageFault`/`DegradeHostObservation`、零 loss 和 fault
`0x45422001` 的 Degraded heartbeat；严格报告写入
`build/ebpf_page_fault_qualification.json`。该结果把“受控 hosted x86_64 匿名页
first-write 计数阈值链”从开放项中移除，但不提供 fault address/IP，不从 BPF
区分 major/minor，不测量 handler 时长，也不证明内存压力、swap/storage、自然负载
根因、产品阈值、生产内核、开销/WCET 或长时 HIL，因此 FR-048 仍保持部分实现。

FR-048 的 network-drop 子路径现已增加独立特权托管 Linux 资格：
`make test-ebpf-network-drop-runtime` 创建唯一 veth 对，关闭可用的接口级 IPv6，
固定到一个 allowed CPU，并只加载/要求 `skb:kfree_skb`，精确过滤接收端 ifindex
与 EtherCAT EtherType `0x88a4`。错误 EtherType 正向帧和 EtherCAT 反向帧必须推进
独立接口 drop counter 但保持 BPF/incident 静默；正式四帧注入必须使接收端
`rx_dropped` delta 至少增加四、BPF 精确记录四次丢包，并只产生唯一
`KernelNetwork/NetworkDrop`、Error/confidence-75 `HostNicDrop`/`ControlledStop`、
零 loss 和 fault `0x45422001` 的 Degraded heartbeat。严格报告写入
`build/ebpf_network_drop_qualification.json`，且仅在 runtime 拆卸、raw socket 关闭、
veth 删除与 affinity 恢复后原子发布。该结果把“受控 hosted virtual-veth
unhandled-EtherType 接收丢包链”从开放项移除，但不证明物理 NIC、driver/NAPI/XDP、
qdisc、队列压力、拥塞、真实 EtherCAT 设备、生产内核、开销/WCET 或长时 HIL，
因此 FR-048 仍保持部分实现。

## 8. 非功能需求

### 8.1 实时性与性能

| ID | 优先级 | 需求 | 验收标准 |
| --- | --- | --- | --- |
| NFR-001 | P0 | 激活后，周期、服务、请求和诊断 API 不得调用 heap、阻塞锁、睡眠或无限轮询。 | allocator wrapper 和 30 分钟混合压力测试中，malloc/free/new 调用次数为 0。 |
| NFR-002 | P0 | Q1 基线应支持 1 ms、最多 32 从站、最多 32 轴加低速 IO、过程映像不超过 1024 B、DC 开启。 | 30 分钟测试中 release jitter P99 <= 10 us、fast path P99 <= 250 us、deadline miss = 0。 |
| NFR-003 | P0 | Q2 伺服应支持 500 us、最多 24 从站、16 轴、过程映像不超过 512 B、DC 开启。 | 30 分钟测试中 release jitter P99 <= 5 us、fast path P99 <= 125 us、deadline miss = 0。 |
| NFR-004 | P1 | Q3 高频支持为 250 us、最多 16 从站、8 轴、过程映像不超过 256 B 的可选平台等级。 | 仅通过独立 30 分钟报告的平台可声明支持；未通过时必须回退为 500 us 或 1 ms。 |
| NFR-005 | P0 | Q4 混合压力下，SDO、诊断和其他非实时负载不能导致 P0 周期 deadline miss，P99 相对空载增加不得超过 10%。 | 1 ms、16 轴、IO、8 个并发 SDO 请求场景满足完整性能门槛。 |
| NFR-006 | P0 | 核心状态资源目标为：32 从站、1 KiB 过程映像、8 帧槽、8 请求、256 trace records 时小于 64 KiB RAM；CoE+DC Cortex-M 构建 `.text + .rodata` 目标小于 128 KiB。 | map file、stack watermark、DMA/arena/ProcBuf 分项报告并记录编译器与构建参数。 |

### 8.2 可靠性与数据完整性

| ID | 优先级 | 需求 | 验收标准 |
| --- | --- | --- | --- |
| NFR-007 | P0 | RX 匹配与计划校验必须有界；未知、旧、重复或损坏帧不得修改已提交输入。 | PCAP 回放、fuzz 和 fault injection 覆盖 index、长度、类型、地址、世代和 WKC。 |
| NFR-008 | P0 | CPU/RT/ISR、DMA 和跨 CPU 数据交换应使用明确所有权、cache 维护和 acquire/release 语义。 | STM32、HPMicro 与 Linux 声明端口均完成 descriptor、cache 和并发压力测试。 |
| NFR-009 | P0 | 64 位时间与序号在 RV32 和 Cortex-M 平台不得发生撕裂读取或隐藏锁退化。 | host TSAN 模型、目标机长测、原子能力报告和竞争测试通过。 |
| NFR-010 | P0 | 故障恢复必须由产品策略明确授权；拓扑变化或 WKC 异常后不得无条件自动恢复运动输出。 | 拔线、掉站、掉电、AL 错误、驱动 fault 与 supervisor 重启 HIL 全部满足预期状态机。 |

### 8.3 可移植性、兼容性与可维护性

| ID | 优先级 | 需求 | 验收标准 |
| --- | --- | --- | --- |
| NFR-011 | P0 | 协议核心应为 Rust `no_std`，不依赖 POSIX、Linux、RTOS 或平台 SDK，且仅通过端口 trait 访问硬件资源。 | host、Cortex-M 和 RV32 交叉编译通过；依赖图无平台违规。 |
| NFR-012 | P0 | Linux raw、一个 STM32 和一个 HPMicro 端口必须提供端口资格材料与同一基础回归用例。 | 每个端口包含板级配置、PHY、descriptor、cache、loopback、HIL 脚本和已知限制。 |
| NFR-013 | P1 | 兼容矩阵应覆盖板卡、PHY、RTOS/内核、从站、驱动固件、PDO、模式、周期、ROS 2 distro、Zenoh 与 RMW 版本。 | 发布前由质量负责人审核矩阵；未验证组合标记为不支持或实验性。 |

### 8.4 安全、合规与许可证

| ID | 优先级 | 需求 | 验收标准 |
| --- | --- | --- | --- |
| NFR-014 | P1 | 所有外部运动命令应具备来源身份、权限、TTL、序号和审计事件。 | 未授权、过期、重放、重复和断连命令测试均被拒绝或按策略降级。 |
| NFR-015 | P0 | 产品文档应明确普通控制与功能安全的边界，不将 ROS 2、Zenoh、CoE/PDO 或普通故障处理宣称为认证安全功能。 | 发布审查中包含安全边界说明；FSoE/安全 PLC 需独立需求与证据包。 |
| NFR-016 | P0 | ESOP 的实现应独立编写，并维持第三方来源、许可证、协议/商标和测试义务的审计边界。 | 发布前完成许可证扫描、第三方清单、代码来源审计和适用的合规评审。 |
| NFR-017 | P0 | MLG 的实时决策路径应为固定容量、无动态分配、无等待且可在构建报告中单独度量。 | 资格报告包含 MLG 的 P99/max 执行时间、状态转换次数和资源占用；不突破场景周期预算。 |
| NFR-018 | P1 | eBPF agent 的资源、权限、内核版本、BTF、attach 点、采样率和丢失事件必须有运行时报告；观测开销必须纳入 Linux 端性能基线。 | 报告包含 agent CPU、内存、ringbuf 使用率、事件丢失、程序运行次数/时间和对 gateway/RT 端到端延迟的影响。 |

## 9. 关键数据与接口契约

### 9.1 ProcBuf

ProcBuf 是实时数据 ABI，而不是通用消息总线。它必须是固定大小、预分配和生成式布局，至少包含：

1. Header：magic、ABI version、layout hash、robot ID、boot ID。
2. Command：序号、命令时效、请求模式、运动使能、关节命令与 IO 命令。
3. State：序号、EtherCAT 时间、实时单调时间、关节状态、IO 状态和总体健康度。
4. Quality：每 Domain WKC、freshness、link、AL、DC、故障位图。
5. Event：固定记录的事件环。
6. Lifecycle：MLG state、门槛位图、motion permit 摘要、首个阻塞原因、请求/执行的停止动作和恢复序号。
7. Runtime observation：最近 incident ID、主机观测健康、观测窗口、eBPF agent epoch 和事件丢失计数。

所有关节值使用 SI 单位或由 profile 明确记录的原始单位与缩放。同一字段不得因驱动品牌而改变语义。

### 9.2 Protobuf 与 Zenoh

1. Protobuf 仅用于控制面和观测面，数据契约置于 `proto/esop/v1/`。
2. 消息应包含适用的 `robot_id`、`boot_id`、schema version、单调时间和 source sequence。
3. Zenoh 推荐命名空间为 `esop/<fleet>/<robot_id>/{state,event,diagnostic,cmd,query}`。
4. Zenoh、Protobuf 和记录服务只从 ProcBuf 状态快照读取数据，不能持有 EtherCAT MAC/DMA 或调用周期 API。

### 9.3 ROS 2

1. 运动控制经 `esop_ros2_control` 接入 `ros2_control`，以 ProcBuf 状态更新 `state_interfaces`，以命令页接收 controller 输出。
2. 系统集成经独立 bridge 实现 ROS 2 topic/service/action 与 ESOP API 的显式映射。
3. ROS 2 使用 DDS 或 Zenoh RMW 是部署选择，不改变 ESOP 实时接口，也不构成功能安全或 EtherCAT 实时保证。

## 10. 平台与部署要求

### 10.1 目标部署

| 模式 | 目标 | 产品定位 |
| --- | --- | --- |
| `split-mcu` | STM32/HPM 实时节点 + Linux 监督节点 | P0 量产默认。 |
| `single-host-dev` | Linux raw port + 同机 ROS 2/Zenoh | P0 开发、仿真与 HIL；不作为 MCU 实时性能替代证据。 |
| `split-linux-rt` | 高性能 ARM SoC/Linux RT 实时节点 | P1，需独立性能资格。 |

### 10.2 端口准入

平台只有在满足下列条件后才可标记为支持：

1. 100 Mbit/s 全双工 MAC/PHY，且可查询链路状态。
2. 可收发 EtherType `0x88A4` 的原始二层帧，不依赖 TCP/IP、ARP 或普通 socket 缓冲路径。
3. 固定 TX/RX descriptor、调用方提供的 DMA 对齐缓冲、可验证 DMA 所有权和缓存一致性。
4. 单调时间源、可配置的周期释放机制、ISR 与周期任务职责隔离。
5. 为目标配置预留过程映像、帧、邮箱、从站表、请求与追踪资源。

## 11. 版本路线与发布门槛

| 里程碑 | 产品交付物 | 出口条件 |
| --- | --- | --- |
| R0：契约与仿真基线 | ProcBuf ABI、MLG 状态/permit 契约、Protobuf v1、设备模型、配置生成、wire/unit、PCAP/虚拟从站。 | 同一配置可生成 C layout、YAML、descriptor 和静态配置；MLG 状态模型/属性测试及 ABI/schema 兼容检查通过。 |
| R1：最小 EtherCAT 实时节点 | 端口、扫描、AL、静态 PDO、单 Domain、WKC、基本诊断。 | 1/8/32 从站达到 SAFEOP/OP；1 小时无内存增长。 |
| R2：伺服与实时资格 | CoE、DC、ProcBuf、MLG、CiA 402 单轴/双轴和 IO、故障策略。 | 两种驱动 + IO 完成 HIL；Q1/Q2、无动态分配、MLG 状态机和故障矩阵证据通过。当前已具备跨层质量门投影、LifecycleSnapshot、双轴独立许可、配置停止动作、新 permit epoch 恢复、ProcBuf v4 生命周期摘要、逐轴请求/成功提交控制字/反馈证明位及原始周期质量位图、固定转换历史发布、有序 lifecycle 转换/逐轴超时事件生成、环满重试及历史覆盖显式确认接口、基于已提交 Domain 的新鲜停机反馈适配器、冻结计划的停机帧提交与跨轴别名拒绝、单 Domain 活动/停机/禁止输出周期分支协调器（真实周期核对、模式/目标限幅及输出 PDO 别名验证、新激活的目标守卫重置并在使能边沿由新鲜实际反馈播种、使能边沿目标与实际反馈一致、成功 TX 后推进目标、活动帧失败同周期撤销许可并尝试停机、失败 TX 仍发布证据、后续新鲜反馈确认、锁存超时后发送 Disable、State 后发布有序事件）、TX/构帧失败后停机证明拒绝与重试及 RX 超时索引复用边界测试、证据绑定能力清单和构建/性能报告校验测试；显式预 RX 过程 Domain 计划已具备激活期绑定、定长提交、阶段证据和生命周期预算投影，稳定周期已具备上一轮输出到下一轮 RX 的固定容量阶段交接、generation/deadline 核对、重复 priming 拒绝及发布后任务结算，统一生产服务调度器已按启动、映射、DC 配置、邮箱固定优先级管理单请求所有权、跨周期等待、未发送请求重建、终态回交、故障阻塞和 MLG 门控投影；可选受控停车规划器已覆盖 CSP Hold、CSV/CST RampToZero、事务性 TX 提交、ProcBuf 实际动作证据，以及 stop-only/共享 RX/控制服务/统一生产服务周期接线与转换序列级回退锁存，仍缺具体产品限幅/HIL 资格、目标硬件 WCET 和实物 HIL 证据。 |
| R3：产品化集成 | 多设备、外设模型、事件、配置报告、Zenoh/Protobuf 网关、ACL、Linux eBPF 运行时观测。 | 网络断连不阻塞周期；命令鉴权/TTL/审计、eBPF 观测健康和 schema 升级测试通过。 |
| R4：机器人软件集成 | `ros2_control`、ROS bridge、URDF/配置生成、双轴轨迹演示。 | `read/update/write` 不绕过 ProcBuf；仿真与实机 HIL 演示及兼容矩阵完成。 |
| R5：扩展与专项 | FoE、其他协议、冗余、FSoE 项目对接、官方流程。 | 每个扩展有独立开关、资源/周期影响报告与专项证据。 |

R2 当前进展：固定容量 `ScheduledDomainBank` 已拥有多 Domain 共享 RX，`ScheduledAuxiliaryOutputs` 按**下一次 RX 周期**的冻结 due mask 提前提交辅助输出，生命周期入口会在 State/事件发布后完成最终 deadline 观测和补救。`ScheduledProcessInputs` 可在启动、恢复或其他明确需要预 RX 发送的阶段完成首次 priming；`ScheduledProductionCycleOwner` 随后以 `PrimingRequired → ReceiveArmed → OutputPending → ReceiveArmed` 固定状态交接上一轮生命周期输出，核对周期、generation、RX deadline 和实际完成报告，拒绝重复 priming，并把完整过程 handoff、最终 deadline、State 与事件发布合并为任务释放证据。`ScheduledProductionServiceScheduler` 现统一选择启动、映射、DC 配置与邮箱控制器，固定优先级为 Startup → Mapping → DC Configuration → Mailbox；调度器持有唯一请求句柄，`Prepared` 最多发送一次，未上线时释放并从原 FSM 动作重建，`InFlight` 跨周期保留且不重发，`Complete`/`Failed` 由匹配 FSM 消费并释放，故障服务保持优先级直到外部显式重启。服务报告直接携带选择、进度、精确故障、恢复状态和就绪值，并由 Bank 核对真实 RX 后映射到 Topology 或 Configuration 门，不再由调用方手工声明 gate/readiness。可选受控停车入口已通过 `ControlledStopCycleState` 接入 stop-only、共享 RX、控制服务和统一生产服务周期；限幅由调用方冻结，受控验证/构帧/TX 首次失败会将当前 MLG 转换序列锁定到默认 Disable/QuickStop，cycle outcome 与 production release 均保留成功使用/回退证据。原始单位限幅、驱动行为、制动器和机械条件仍需逐产品冻结并通过 HIL。目标硬件 WCET 与实物 HIL 验收仍未完成，上表 R2 出口条件仍未满足。

R2 资格门当前进展：新增 `qualification/r2_qualification.json`、独立 HIL topology manifest、已知限制和 `validate-r2-qualification.py`。声明通过时必须把逐轴 `ControlledStopLimits` 同构字段、两厂商驱动与 IO 兼容组合、CSP/CSV/CST 和配置停止动作、故障矩阵、目标构建资源、Q1/Q2 HIL 性能及安全/许可证/来源审查绑定到仓库内路径和 SHA-256；build 与性能报告还必须属于同一 tested commit/config，性能报告必须匹配完整 topology hash。当前基线故意保持未合格，因此只封闭证据声明路径，不改变实物 HIL 和目标 WCET 的未完成状态。

R2 后续增量：可选的实测 deadline 入口在活动 TX 前后采样端口时钟。发送前超时阻止活动输出；发送后超时立即撤销许可并发布预算失效/Stopping，区分已提交活动帧与尚未发出的停机帧；下周期在 RX 索引可复用后再发送停机。此证据只覆盖当前分支的 TX 结束时点，尚未覆盖 State/事件发布、其他 Domain TX、完整周期终点及实物 HIL，因此不改变 R2 出口门槛。

R2 多 Domain RX 增量：固定容量 `ScheduledDomainBank` 将冻结 ID 顺序和独占数据报索引绑定到实际 Domain；每周期仅接收到期 Domain，并在漏收时使其质量失效。软件模拟已验证真实辅助 Domain 到期漏收会撤销运动许可，且运动 Domain 的已提交输入与真实质量由同一接收所有者提供给生命周期停机分支。发送计划、辅助 Domain 输出、DC/控制接收以及最终 deadline 仍由完整周期所有者接线；本增量不满足 R2 的 HIL 与实时资格出口条件。

R2 辅助 TX 增量：可选的 `ScheduledAuxiliaryOutputs` 在激活时将运动与辅助分帧计划绑定到 `ScheduledDomainBank` 的真实段，拒绝索引复用、镜像越界及可写地址重叠。生命周期输出阶段按下一次共享 RX 的周期计算 due mask：周期 N 结束时提交周期 N+1 到期的辅助与运动帧，避免 period > 1 的 Domain 发生一拍相位错位。首次 TX 失败或发送跨过当前周期截止时间时，同周期撤销运动许可并尝试发送停机帧，State 保留已接受帧数量、失败位置和预算结果。软件模拟覆盖提前一拍提交、非到期不发、下一拍到期漏收、TX 失败与跨期截止时间；安全镜像来源、其他服务和实物 HIL 仍未闭环，R2 出口条件不变。

R2 同周期 DC RX 增量：固定容量 `ScheduledDomainBank::receive_with_dc` 将到期 Domain 和 DC 响应交由同一次主站 RX 分发；DC 索引冲突/世代不匹配在进入接收前拒绝。即使 DC 响应丢失或端口 RX 报错，也结束到期 Domain 和 DC pending；端口错误返回保守预算失败的真实周期报告和原始错误。软件模拟验证 DC 缺帧而运动 Domain WKC 有效时同周期停止、旧 DC lock 不复用、恢复同步后不自动恢复旧许可、端口错误的保守质量。调用方仍需负责 DC 准备/发送、控制接收、TX 计划、安全镜像、完整周期最终 deadline 及实物 HIL；先前段落中的“DC 接收未接线”仅描述当时阶段，R2 出口条件仍未满足。

R2 控制 RX 增量：`receive_with_dc_and_control` 在同一有界主站 RX 中接收到期 Domain、DC 与在途控制请求，前置检查拒绝与 Domain/DC 或另一控制请求复用索引。控制请求完成时再次核对数据报索引和命令，防止共享帧槽的无关数据报误投递。固定容量模拟测试覆盖三类同周期响应、仅丢控制响应时请求仍待处理及冲突在接收前拒绝。调用方仍负责控制请求期限、状态机结果与安全事实、DC/控制 TX、最终周期 deadline、实物 HIL；该增量不能视为完成 R2 出口。上述早期阶段的“控制接收未接线”不代表当前状态。

R2 控制超时增量：`ControlRequestPool::expire_in_flight` 在 RX 完成后以单调时间扫描最多 64 个在途请求，仅在 `now_ns > deadline_ns` 时一次性置为 `Failed(Timeout)` 并返回固定容量句柄位图；准时完成、Prepared 和既有失败不被覆盖，迟到响应不得擦除超时诊断。调用方须在主站回收同期限 RX 索引后消费超时句柄，并为每个请求调用所属服务状态机再释放；邮箱 `accept_completed` 已将匹配的超时请求接入现有重试预算，耗尽则锁存 `MailboxError::Timeout`。模拟端验证缺帧、索引回收与请求失败一致。本增量仍需完整周期所有者实际调用超时扫描并推进其他控制服务；安全事实、DC/控制 TX、最终 deadline 与实物 HIL 尚未闭环，R2 出口条件不变。

R2 控制服务 RX 接线增量：`receive_with_dc_and_control` 已在本次 RX、Domain/DC 收尾完成后，以同一个端口单调时间自动将新过期控制请求放进 `ScheduledReceiveReport.control_expiry`；有新过期请求时，返回前回收其主站 RX 索引。链路断开和端口收包错误的提前返回同样执行该收尾；无新过期请求时不额外扫描 256 个 RX 索引。三周期模拟覆盖发送确认、轮询缺帧、后续同源接收超时与邮箱延迟重试，同时保持 Domain/DC 接收资格；预检拒绝仍不改变请求状态。调用者仍负责消费完成/失败请求、配置安全事实、控制/DC 发送和完整周期最终 deadline，软件模拟不替代实物 HIL，R2 出口未达成。

R2 接收结果与输出绑定增量：`StopCycleContext::run_received_with_outputs_until` 在发送辅助或运动帧之前，核对共享 RX 报告是否属于当前 `ScheduledDomainBank` 刚结束的周期、真实 Domain 质量、当前主站报告、运动 Domain 实例及 DC 收尾结果，并按冻结调度顺序生成生命周期质量快照。模拟集成测试让运动 Domain、IO Domain、DC 和控制请求在同一次共享 RX 完成，随后发送到期辅助与运动输出；修改报告质量或周期均在新 TX 前拒绝。下一周期即使运动 Domain 正常，到期 IO 与 DC 缺帧仍撤销许可并进入停止，旧报告不可复用。此入口仍依赖外层提供非总线安全事实、DC/控制 TX 与安全镜像；传入绝对 deadline 时会完成 State/事件发布后的最终采样和补救，但不覆盖过程帧提交前后的完整任务释放，也不替代双厂商伺服与 IO 实物 HIL 资格。

R2 发布后 deadline 观测增量：受检输出入口在首次 State/事件发布尝试结束后再次采样端口单调时钟，单独返回 `post_publication_deadline_met`。若 TX 后仍在预算内、发布后才越界，则同周期撤销许可、锁存预算故障、清除 CycleBudget 质量并尝试第二次发布更正 State，再发送新转换事件；`deadline_correction_publish` / `deadline_correction_events` 将更正失败显式暴露，已接受的活动帧不会冒充停机帧。此前首次 State 或 Active 事件可能已经被并发读者观测，无法撤回；消费者不能单靠首次发布判定整个周期合格。外层周期所有者仍须管理全部 DC/控制发送、非总线安全事实与任务释放、对更正失败执行安全升级，并完成真实目标平台的 WCET/截止时间资格验证。因此本增量仅补充观测和补救，不宣称全周期原子发布或满足 R2 出口。

R2 DC/控制发送增量：`ScheduledDomainBank::submit_dc_and_control` 在一次共享 RX 的准备阶段预检当前周期、绝对期限、DC/控制世代与准备态，以及与所有 Domain 和在途控制请求的索引独占关系；失败时不发送帧。通过预检后依次准备并提交 DC 帧和最多一个控制帧，保留端口接受情况、首个构帧/发送失败和 TX 后期限观测。发送失败的控制请求立即进入带 `TransmitFailed` 原因的终态；若 DC 已准备，即使发送失败，也必须由共享 RX 入口收尾以清除 pending，不能沿用旧同步证据。DC 发送失败或周期期限中断后未发的控制请求仍为 `Prepared`，由所属服务显式处理。帧槽或 DMA 描述符可先于旧响应回收，因此 TX 失败仅撤销本次帧实际武装的 RX 索引，不得按可复用槽编号清除先前已接受帧的预期。软件模拟和 DMA 回归覆盖此边界。调用方仍需将发送失败和期限结果接入安全事实，消费控制请求结果，发送到期过程输出，完成发布后周期结算和实物 HIL；不宣称 R2 已验收。

R2 邮箱服务收尾增量：`receive_with_dc_and_mailbox` 在接收前验证传入请求与当前邮箱动作的数据报索引、世代、地址、操作、长度、期限及待发负载一致；读/写数据报在线上均可能有补零占位，必须同时校验原负载前缀与零填充，不允许外来请求使邮箱状态机故障。完成/失败态的缓冲已被响应覆盖，不再比较原负载。随后使用同一有界 RX 完成到期 Domain/DC/控制请求，并在完成或失败时立即让邮箱状态机消费和释放该请求。未响应但未过期的 `InFlight` 请求仍保留，跳过发送的 `Prepared` 请求也不被冒充已发送；缺帧到期按原有限次数重试，TX 拒绝则以 `Control(TransmitFailed)` 进入同一延时重试/耗尽路径。返回值同时保留完整 RX 质量、控制超时位图及显式的邮箱结果，调用方仍要将失败接入安全事实。模拟覆盖成功推进、在途等待、缺帧超时、误绑定拒绝和 TX 失败后延时重试。启动与其他控制服务、过程输出发送、周期最终 deadline、设备 HIL 和完整生产周期所有者仍未闭环，R2 出口不变。

R2 邮箱服务发送增量：`submit_dc_and_mailbox` 将邮箱动作选择、固定请求池申请、DC/控制顺序发送及未发送请求回收收进同一有界入口，并与上述共享 RX 收尾形成显式句柄契约。调用方把上一周期返回的请求句柄原样传回：`Prepared` 请求可发送，`InFlight` 请求只保留且不重复上线，`Complete`/`Failed` 必须先由 RX 入口消费。DC 失败或期限中断使新请求仍处于 `Prepared` 时，入口释放池槽但保留邮箱动作供后续重建；调用方已有请求若预检失败则保持所有权，不被静默释放。模拟覆盖自动申请、成功推进、跨周期在途等待、控制响应超时重试、控制 TX 失败重试、DC TX 失败回收及预检错误保留调用方请求。完整周期所有者仍需调度 Domain 输出、映射发送与邮箱失败到安全事实、完成发布后最终 deadline，并取得实物 HIL 证据；R2 出口条件不变。

R2 邮箱服务周期所有者增量：`run_dc_and_mailbox_cycle` 在调用方已提交本世代过程 Domain 帧后，以一个固定容量入口原子编排邮箱/DC 服务发送、共享 Domain/DC/控制接收、请求终态消费和跨周期句柄更新。服务发送一旦通过预检，即使 DC 或控制 TX 报告失败也继续执行共享 RX 收尾，防止已准备 DC 世代泄漏到下一周期；完成/失败的邮箱请求由服务状态机消费后不再返回句柄，只有仍在途请求可继续携带。返回值同时保留 TX 报告、完整 RX 质量、邮箱进度和 RX 完成后的阶段 deadline；内部 RX 预检异常则连同 TX 报告返回并要求故障/重新初始化。仿真覆盖正常推进、丢失控制响应后的跨周期在途持有与超时重试、RX 结束恰逢 deadline，以及 DC TX 失败仍清除 pending。该早期入口本身尚未发送过程输出、映射服务错误到 MLG 非总线事实、发布 State/事件或判定发布后的最终 deadline；这些职责已由后续生命周期入口、稳定周期所有者和统一服务调度增量补齐，实物 HIL 仍未完成，R2 出口条件不变。

R2 服务周期到生命周期接线增量：`StopCycleContext::run_mailbox_cycle_with_outputs_until` 直接消费上述完整 `ScheduledMailboxCycleReport`，在门槛更新或新过程 TX 前核对跨阶段请求句柄、邮箱终态、DC/控制发送形状以及 TX/RX deadline 关系，拒绝调用方拼接的矛盾报告。任一服务 TX 失败、邮箱错误或 `RetryScheduled` 会保守清除本周期 CoE/配置资格；RX 后阶段 deadline 失败会清除且不能恢复调用方预算事实。验证通过后，该入口从报告中的同一共享 RX 推导冻结 Domain 质量，发送到期辅助/运动输出，发布 State/事件，并执行发布后的最终 deadline 观测与现有补救。软件回归覆盖正常路径、报告篡改拒绝及服务失败、重试、超时、阶段越界映射。该早期入口本身仍要求外层先提交过程 Domain 帧并负责完整任务释放、其他服务与调度结算；后续稳定周期所有者、统一服务调度和受控停车生产接线已补齐这些软件边界，目标硬件 WCET 和双厂商伺服加 IO 实物 HIL 仍未完成，R2 出口条件不变。

R2 显式预 RX 过程提交增量：`ScheduledProcessInputs::new` 在激活期把冻结调度中的每个 Domain 与其只读过程镜像和可分帧计划逐一绑定，拒绝 Domain 顺序错配、段覆盖不完整、镜像越界、索引复用及跨 Domain 可写地址重叠。`ScheduledDomainBank::submit_due_process_inputs` 只遍历当前到期项，返回逻辑周期、generation、RX deadline、到期掩码、预期/已接受帧数、首个失败位置和 TX 后 deadline 观测；端口拒绝会撤销该帧 RX 预期，随后共享 RX 仍可把缺失 Domain 收尾为无效。`StopCycleContext::run_process_mailbox_cycle_with_outputs_until` 同时核对该阶段报告和完整邮箱/DC 周期报告，任何过程帧失败或阶段超时都会在辅助/运动 TX 前清除 CycleBudget 资格，篡改周期、generation 或计数则直接拒绝。该入口只用于首次启动、恢复或其他明确未由上一轮输出武装 RX 的阶段；稳定循环由 `ScheduledProductionCycleOwner` 保存不可变 `ScheduledProcessHandoff`，先核对实际 RX，再等待同周期生命周期输出结算为下一代 handoff，禁止重复调用预 RX 提交器；下一代 RX deadline 还必须与调度器提供的期望绝对值完全一致。任务释放仅在下一代过程帧完整、TX/发布后最终 deadline 合格、State 与事件均发布成功时成立；失败或部分提交仍进入下一次 RX 排空已武装索引。其他服务统一编排、完整错误恢复、目标硬件 WCET 和实物 HIL 仍待完成，R2 出口条件不变。

R2 非邮箱控制服务周期增量：`ScheduledDomainBank::run_dc_and_control_cycle` 接受由启动、映射或 DC 配置状态机创建并持续持有的单个请求句柄，记录发送前与共享 RX 后的不可伪造状态证据。只有 `Prepared` 请求会上线；已有 `InFlight` 请求不重复发送，只参与同一有界 Domain/DC/control RX、严格期限超时和索引回收。提交通过预检后即使 DC/控制 TX 报告失败也必须完成共享 RX 收尾；完成或失败请求不由传输层释放，而是原样交回所属服务 FSM 校验动作身份、推进业务状态并释放池槽。`ScheduledControlGate` 要求调用方明确声明服务拥有 Platform、Configuration、Topology 或 Drive 中的哪一门，`run_control_cycle_with_outputs_until` 与 priming 变体会把服务 FSM 的真实就绪状态和传输终态合并，任何 TX/请求失败都不能被调用方 `true` 恢复；稳定周期所有者也只接受 Bank 核验通过的控制周期。跨 crate 仿真使用真实 `MappingConfigController` 覆盖成功推进、丢响应后跨周期不重发、严格超时故障、门控投影，以及上一轮过程输出 + 新控制/DC 共享 RX 后的停机路径。下一步仍需把邮箱与各控制器的选择、请求重建、故障升级和任务释放收进一个生产调度器；本增量不替代目标硬件 WCET 与实物 HIL。

R2 统一生产服务调度增量：`ScheduledProductionServiceScheduler` 在固定容量周期路径中统一接管 Startup、Mapping、DC Configuration 和 Mailbox 的选择与单请求所有权。调度器只允许当前最高优先级服务持有请求；请求池条目必须与该 FSM 的不可变 pending action 在 index、generation、地址、操作、长度、期限和待发负载上完全匹配。控制请求允许以自身已武装 generation 跨后续生产周期等待，但只有 `ControlRxConsumer` 能在同一 InFlight 槽位和数据报索引仍匹配时授权该旧 generation；Domain/DC 仍严格使用当前周期 generation。DC/期限失败留下的 Prepared 请求会被释放并标记 `RebuildRequest`；若原动作在重建前已过期，则不再申请池槽而直接由所属 FSM 消费 Timeout，提交预检拒绝也只释放尚未上线的 Prepared 句柄。丢响应的 InFlight 请求保持 `AwaitingResponse` 且后续周期不重发，终态由所属 FSM 以原始 `ControlError` 消费，故障服务阻止低优先级邮箱直到外部重启。`ScheduledProductionServiceCycleReport` 由 Bank 校验底层控制或邮箱周期，`StopCycleContext` 的统一入口直接把 Startup 映射到 Topology、Mapping/DC/Mailbox 映射到 Configuration，并可由稳定周期所有者完成 RX 阶段交接。软件模拟覆盖跨 generation 完成、固定优先级、故障阻塞/显式重启、Prepared 重建及过期回交、提交拒绝清理、InFlight 无重发和严格超时释放；可选受控停车现可由同一统一生产报告入口执行并把成功/回退证据交给周期所有者，逐产品资格、目标硬件 WCET 与实物 HIL 仍是 R2 发布阻塞项。

R2 受控停车生产接线增量：固定状态 `ControlledStopPlanner` 与 `submit_controlled_stopping_frame` 不改变默认 `step_axis_bank` 将 Hold/Ramp 降级为 Disable 的生产行为。Hold 仅在 CSP 下锁定停车序列第一份已验证实际位置；RampToZero 仅在 CSV/CST 下按冻结的原始单位每周期步长，从当前已验证速度/转矩反馈向零收敛。动作、模式、限幅和 MLG 转换序号在序列中不可变化；输入必须来自同周期完整 Domain，下一代 Domain 只能在成功发送后武装。所有规划先在副本预演，只有端口接受完整帧才提交规划器和 `stop_issued_cycle`。新增调用方持有的 `ControlledStopCycleState` 冻结逐轴限幅，并通过 `run_with_controlled_stop`、共享 RX、控制服务及 `run_service_cycle_with_controlled_stop_until` 接入现有固定容量周期路径；首次映射缺失、模式不符、输入不可验证、周期/序列重放、跨轴别名、构帧或 TX 失败会重置规划器，把当前 MLG 转换序列锁定到默认 QuickStop/Disable，防止后续周期重新启用已失败的受控目标，新的转换序列才允许重新规划。`controlled_axis_stops_to_procbuf` 区分受控 Hold/Ramp 目标与终端 Disable，`StopCycleOutcome` 保留原始受控错误与回退帧结果，`ScheduledProductionRelease` 保留成功使用/回退证据；任务释放仍以 handoff、deadline 和发布完整性为准，要求受控策略成功的产品必须额外检查该证据。软件模拟覆盖成功 Hold、输入失败后同序列不重试、默认回退、后续新鲜反馈确认及 owner 结算；逐产品缩放/限幅、两种驱动与 IO 实物 HIL、制动器/安全链验证和目标硬件 WCET 仍是 R2 发布阻塞项。

R2 资格证据门增量：验证器先校验所有证据路径仍位于仓库根目录并匹配 SHA-256，再复用现有 build/performance 校验语义。任何占位平台、缺失/漂移证据、逐轴限幅不完整、厂商或 IO 数量不足、模式/停止动作覆盖不足、报告未通过、commit/config/topology 不一致或审查缺失都会拒绝 `passed: true`。CI 输出 `build/r2_qualification_report.json`；该规范化结果只证明证据包一致性，不能生成 HIL trace、WCET 或审批。

### 11.1 发布阻塞条件

任何发布候选必须满足：

1. 所有承诺的 P0 需求均有自动化、仿真或 HIL 可追溯证据。
2. 目标板、拓扑和周期下无未解释的 timeout、WKC 错误、DMA 错误或内存增长。
3. `capability_manifest.json`、`robot_build_report.json`、`performance_report.json`、HIL topology manifest 和已知限制齐全。
4. 仅声明兼容矩阵中已通过的驱动、模式、板卡、PHY、RTOS/内核与软件版本组合。
5. 许可证、代码来源和安全边界审查完成；不作超出证据的认证或性能宣传。

## 12. 测试与验收策略

| 层级 | 覆盖内容 | 最低要求 |
| --- | --- | --- |
| Wire/unit | 帧、数据报、PDO 位域、FSM、WKC、CoE、CiA 402、ProcBuf、MLG、无锁原语。 | 每项 P0 协议/状态需求有正常与异常测试；MLG 的状态迁移和门槛组合可穷举。 |
| 仿真/回放 | 虚拟从站、PCAP、乱序、重复、丢失、坏长度、旧帧、WKC 异常。 | 可在 host CI 复现，并校验状态不会被错误提交。 |
| 端口资格 | MAC/PHY、descriptor、cache、IRQ、DMA、loopback、帧注入。 | Linux、STM32、HPMicro 各通过基础端口用例。 |
| HIL 基础 | 1/8/32 从站、扫描、配置、OP、PDO、SDO、DC。 | 至少两厂商 CiA 402 驱动和一种 IO 模块。 |
| HIL 故障 | 拔线、掉电、AL/WKC/DC、驱动 fault、邮箱 abort、IPC/ROS/Zenoh 重启、permit 过期、外部 safety inhibit。 | 每种故障验证 MLG 状态、事件、质量、停止动作和恢复条件；Linux 端额外验证 eBPF 归因。 |
| 性能/Soak | Q1/Q2/Q3/Q4、其他 DMA 压力、控制面负载、eBPF 观测负载、温度。 | Q1/Q2 至少 30 分钟，候选发布建议 8 小时；报告 P50/P99/P99.9/max、观测开销和事件丢失。 |

## 13. 风险与缓解

| 风险 | 影响 | 产品缓解策略 |
| --- | --- | --- |
| 250 us 周期在某板卡/拓扑上不可达 | 无法满足高频产品定位 | 以 500 us 或 1 ms 为已验证降级等级；不得改变统计口径宣称通过。 |
| 静态计划限制在线调整 | 拓扑或映射变更需要停机 | 进入受控安全状态后重新配置并激活，不支持无条件在线重映射。 |
| RX 完整性校验增加 RAM/拷贝 | 小 MCU 资源压力 | 通过生成布局和资源报告约束；优先保证数据正确性。 |
| 控制面长期饥饿 | SDO/恢复延迟 | 采用显式请求 deadline、`DEFERRED/TIMEOUT` 结果与维护模式。 |
| Linux 通用网络路径抖动 | 误把功能端口当性能证据 | 将 Linux raw 标为开发/HIL；量产资格使用合格的 MCU DMA 或独立性能端口。 |
| 驱动厂商差异 | CiA 402 对象、缩放、模式行为不一致 | 使用 capability/quirk 描述、双厂商 HIL 和兼容矩阵，不在核心硬编码厂商逻辑。 |
| 外部命令或网关故障 | 未授权/过期命令影响控制 | 来源身份、ACL、TTL、序号、审计与 ProcBuf 时效校验。 |
| 安全需求被普通通信替代 | 合规和人身风险 | 明确普通控制边界；STO、FSoE、安全 PLC 和认证验证单独立项。 |

## 14. 依赖与假设

### 14.1 外部依赖

1. 支持原始二层 Ethernet、DMA 和缓存维护的目标板卡与 BSP。
2. 具备可用 ESI、对象字典、PDO 映射、单位/缩放和固件版本资料的 EtherCAT 从站。
3. HIL 实验条件：真实伺服、IO、线缆、PHY、供电、故障注入与测量工具。
4. Linux 监督域中可用的 ROS 2、`ros2_control`、Zenoh 和 Protobuf 工具链。
5. EtherCAT、ETG、CiA 以及第三方源码/许可证的适用许可与合规咨询。

### 14.2 当前假设

1. 产品首发采用 `split-mcu` 双域部署。
2. 首个机器人使用 2-8 轴 CiA 402 关节与 EtherCAT IO。
3. 1 ms 是首发基线周期；500 us 是 P0 验证目标；250 us 是合格平台的可选等级。
4. 用户所述 `proctbuf` 按 Protobuf 解释；实时共享缓冲统一命名为 ProcBuf。
5. 默认只支持单主站、单活动端口、静态拓扑和静态 PDO 映射。

## 15. 待产品确认事项

以下问题在 R0 结束前必须有明确决策；它们会影响容量、配置生成、HIL 计划和兼容矩阵，而不改变本 PRD 的实时隔离原则。

| 事项 | 需要确认的内容 |
| --- | --- |
| 目标硬件 | STM32/HPMicro 的具体型号、评估板、PHY、MAC/DMA/caching 方案与 RTOS 选择。 |
| 机器人配置 | 轴数、IO 数量、实际 PDO 字节、线缆长度、拓扑、周期、控制带宽和电源/热条件。 |
| 驱动与 IO | 首发厂商、固件版本、ESI、对象字典、CSP/CSV/CST 支持、单位与缩放、厂商 quirks。 |
| 命令策略 | 控制权仲裁、motion permit 的可信来源、TTL、hold/ramp/disable 默认行为、quick stop 选项及 fault reset/MLG 恢复授权条件。 |
| 上层软件 | ROS 2 distro、是否使用 `rmw_zenoh_cpp`、Zenoh router 部署、URDF/控制器和远程访问需求。 |
| 安全范围 | STO、急停、安全 PLC、FSoE、法规/认证边界和安全证据所有者。 |
| 商业与合规 | 开源/商业许可证策略、EtherCAT 商标与官方一致性测试的目标与预算。 |

## 16. 需求追溯与文档责任

本 PRD 是产品需求的主入口。以下文档承担补充责任：

| 文档 | 责任 |
| --- | --- |
| `ethercat-master-requirements.md` | EtherCAT 主站协议能力、端口契约、核心约束与细粒度验收需求。 |
| `robotics-esop-software-plan.md` | 机器人部署、ProcBuf、设备/profile、Zenoh、ROS 2 与产品集成边界。 |
| `esop-performance-architecture-decision.md` | 实时性能决策、预算、指标、性能报告和回退规则。 |
| `esop-etg-cia402-master-requirements.md` | ETG.1500 Class B、Motion Control Feature Pack、CiA 402、无锁交接与一致性边界。 |
| `esop-motion-lifecycle-guard.md` | MLG 状态机、门槛模型、停止/恢复策略、数据契约和测试证据要求。 |
| `esop-ebpf-runtime-observability.md` | Linux eBPF agent、内核/用户态观测点、事件关联、RuntimeIncident 和观测安全边界。 |

对每个实现里程碑，需求 ID、测试用例、HIL 场景、报告 hash 和兼容矩阵项必须可相互追溯。任何改变 P0 范围、性能门槛、故障策略、支持声明或安全边界的变更，都必须更新本 PRD 并重新评审相应验收证据。
