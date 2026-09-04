# ESOP 软件产品需求文档（PRD）

- 产品：ESOP（EtherCAT Simple Operating System）
- 文档版本：1.1
- 日期：2026-09-03
- 状态：规划基线
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
| FR-048 | P1 | eBPF 应观测调度、IRQ/softirq、网络收发/丢弃、页错误、OOM/进程退出、CPU 迁移/限频和 ESOP/ROS/Zenoh 用户态关键函数。 | 至少能识别 scheduler stall、IRQ storm、NIC drop、page fault、CPU throttle、process crash 和 gateway stall。 |
| FR-049 | P1 | 观测事件应与 ESOP `boot_id`、cycle sequence、组件 PID/TID、CPU、网卡、ProcBuf transition sequence 和 monotonic time 关联。 | 一次周期异常可以从 `performance_report` 追溯到对应的 eBPF 事件窗口和组件。 |
| FR-050 | P1 | agent 应在内核侧优先聚合计数/直方图，仅在触发阈值或诊断窗口内发送固定大小事件；事件传输不得阻塞被观测进程。 | 高频调度/网络压力下 ringbuf 丢失计数可见，agent 不等待、不向 RT 线程注入锁或同步调用。 |
| FR-051 | P1 | 系统应生成结构化 `RuntimeIncident`，包含 incident ID、级别、原因码、时间窗口、证据、关联周期、影响组件、丢失计数和建议动作。 | 运维界面/Zenoh/Protobuf 能按 incident ID 聚合同一问题的多条证据，而不是只显示孤立日志。 |
| FR-052 | P1 | eBPF 观测只能向 MLG 提供 `HOST_OBSERVATION` 证据或监督心跳，不能直接修改 CiA 402 controlword、绕过 MLG 或成为认证安全通道。 | agent 停止、事件误报、恶意事件和观测延迟测试中，运动许可仍只由 MLG 与产品安全策略裁决。 |

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
| R2：伺服与实时资格 | CoE、DC、ProcBuf、MLG、CiA 402 单轴/双轴和 IO、故障策略。 | 两种驱动 + IO 完成 HIL；Q1/Q2、无动态分配、MLG 状态机和故障矩阵证据通过。 |
| R3：产品化集成 | 多设备、外设模型、事件、配置报告、Zenoh/Protobuf 网关、ACL、Linux eBPF 运行时观测。 | 网络断连不阻塞周期；命令鉴权/TTL/审计、eBPF 观测健康和 schema 升级测试通过。 |
| R4：机器人软件集成 | `ros2_control`、ROS bridge、URDF/配置生成、双轴轨迹演示。 | `read/update/write` 不绕过 ProcBuf；仿真与实机 HIL 演示及兼容矩阵完成。 |
| R5：扩展与专项 | FoE、其他协议、冗余、FSoE 项目对接、官方流程。 | 每个扩展有独立开关、资源/周期影响报告与专项证据。 |

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
