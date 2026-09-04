# 跨平台 EtherCAT 主站需求与架构说明

- 文档版本：0.2
- 日期：2026-09-03
- 状态：基线需求，供架构、实现和测试共同使用
- 目标产品：`ESOP`（**EtherCAT Simple Operating System**）；本文的“主站”均指 ESOP 的 EtherCAT MainDevice/Master 核心组件。
- 能力目标：按 **ETG.1500 Class B v1.0.2 + FP Motion Control v1.0.0** 设计；通过适用的官方一致性测试前不得宣称认证或正式 conformance。
- 详细对照：[ETG.1500、Beckhoff 与 CiA 402 主站决策](esop-etg-cia402-master-requirements.md)

## 1. 目标与结论

### 1.1 产品目标

构建一个可运行于裸机、RTOS 和 Linux 用户态的 EtherCAT 主站核心。它必须比 SOEM 和 IgH 的完整部署更容易移植、更少依赖，并在 MCU 的 DMA 以太网 MAC 上保持可预测的周期通信性能。

首要部署对象是：

- STM32：ARM Cortex-M 系列，使用片上 Ethernet MAC、PHY 和 DMA；
- HPMicro：通常为 RISC-V MCU，不能将“支持 HPM”误写成“只支持 ARM”；
- 通用 ARM：Cortex-M、Cortex-A 裸机/RTOS 或 Linux 用户态；
- 后续可由相同端口接口支持其他具备原始二层收发能力的 MCU/SoC。

核心不依赖 Linux 内核、POSIX、socket、IP 协议栈、C++ 运行时或动态内存分配器。Linux、FreeRTOS、Zephyr、STM32 HAL/LL、HPM SDK 只能存在于平台端口或示例中，不能反向进入协议核心。

### 1.2 设计结论

| 对比项 | SOEM | IgH EtherCAT Master | ESOP 决策 |
| --- | --- | --- | --- |
| 定位 | 轻量 C 库、嵌入式实时通信 | Linux 内核主站及完整运维生态 | 轻量、无内部线程、面向 MCU 的 Rust `no_std` 核心 |
| 传输方式 | OSAL + 每平台 `nicdrv`，直接收发帧 | 内核模块、原生/通用网卡驱动接口 | 独立的 DMA/原始帧端口接口；首版单网口 |
| 配置模型 | 固定容量数组、上下文对象、阻塞式辅助调用较多 | Domain、PDO 注册、异步请求、FSM | 编译期容量 + 调用方提供内存；保留 Domain 和非阻塞请求 |
| 周期数据 | `send_processdata`/`receive_processdata` | Domain 零拷贝、queue/send/receive/process | 双缓冲或 DMA 直连过程映像，显式 cycle API |
| 状态与恢复 | 应用样例通常承担监控/恢复策略 | 主站 FSM、从站 FSM、自动扫描/配置 | 核心提供状态机和事件；恢复策略由应用显式选择 |
| 邮箱协议 | CoE、FoE、EoE、SoE | CoE、FoE、EoE、SoE、VoE | CoE/SDO 为首版必需；其他协议为独立插件 |
| 平台依赖 | Linux/Windows/RTK OSAL、网卡实现 | Linux 内核、网络设备、字符设备、RTDM | 核心 Rust `no_std`；平台层仅 1 个小型 port trait |
| 不应继承的复杂度 | 全局兼容 API、传输层中的同步等待 | 内核模块、私有网卡驱动、ioctl、CLI、TTY、RTDM | 不引入内核态、驱动 fork、管理工具和后台服务 |

SOEM 应作为“协议覆盖和简洁 API”的参考，而不是复制对象；IgH 应作为“Domain、零拷贝、请求队列和显式 FSM”的参考，而不是移植对象。新实现必须独立编写，禁止复制两者的源代码、结构体布局、注释或测试向量中受版权保护的表达。

## 2. 参考实现的模块分析

### 2.1 SOEM

本分析基于官方仓库提交 `2f73eaa803f91f8332b5c8b047ba03a1210c9a80`（2026-06-12）。SOEM 的公开 CMake 目标只由 9 个协议源文件组成，并将操作系统适配放在 `osal/`、网卡收发放在 `oshw/`。这正是其依赖较少、易嵌入的主要原因。

| SOEM 模块 | 主要文件 | 职责 | ESOP 处理 |
| --- | --- | --- | --- |
| 类型、帧和寄存器定义 | `ec_type.h`、`ec_base.h` | EtherCAT 帧、数据报、ESC 寄存器、字节序定义 | 必需；用独立定义重新实现，并对所有线缆格式做单元测试 |
| 数据报访问 | `src/ec_base.c` | APRD/APWR/APRW、FPRD/FPWR/FPRW、BRD/BWR/BRW、LRD/LWR/LRW、ARMW/FRMW | 必需；改为非阻塞的“构帧-提交-完成”引擎，禁止核心同步轮询 |
| 主控与发现 | `src/ec_main.c` | 上下文、原始帧发送/接收、从站寻址、AL 状态、SII/EEPROM、邮箱池、过程数据收发 | 拆分为 `core`、`scan`、`al`、`sii`、`mailbox`；不保留全局 API |
| 配置和过程映像 | `src/ec_config.c` | 扫描、SM/FMMU、PDO 映射、逻辑地址、分组映像、恢复/重配置 | 必需；采用静态配置描述和 Domain，避免运行期隐式分配 |
| 分布式时钟 | `src/ec_dc.c` | DC 拓扑/延迟测量、SYNC0/SYNC1 配置 | 必需，作为可关闭的核心特性；周期路径不得阻塞 |
| CoE | `src/ec_coe.c` | SDO 读写、PDO 映射、对象字典查询 | SDO 上传/下载及 PDO 配置必需；对象字典浏览为诊断可选项 |
| FoE | `src/ec_foe.c` | 文件上传、下载、固件更新 | 可插拔，首个扩展协议 |
| EoE | `src/ec_eoe.c` | EtherCAT 内承载以太网、IP 参数、分片 | 可插拔，不进入实时数据面 |
| SoE | `src/ec_soe.c` | 驱动 IDN 读写和映射 | 可插拔，面向伺服设备的后续能力 |
| 诊断 | `src/ec_print.c` | 错误栈到文本 | 必需但实现为结构化错误码和固定环形追踪，不在实时路径格式化字符串 |
| OS 适配 | `osal/{linux,win32,rtk}` | 单调时间、睡眠、线程、互斥锁、堆 | 拆小；首版核心只需要单调时钟、临界区、DMA 缓存维护钩子 |
| 网卡适配 | `oshw/{linux,win32,rtk}` | NIC 初始化、帧发送、接收、冗余端口 | 改为面向 descriptor 的 `port_ops`；单端口首发，冗余端口预留接口 |

SOEM 的可取之处：单一 C 库、上下文参数化 API、可配置上限、协议覆盖完整。其不适合直接照搬之处：传输层的 `ecx_srconfirm` 同步等待、OSAL 含线程/堆/休眠、固定数组和平台网卡逻辑相互可见，以及兼容旧全局 API 与 `ecx_` API 两套表面。

### 2.2 IgH EtherCAT Master

本分析基于官方仓库提交 `650888c587b1aa570c4d7211adaf39145d9e5ae3`（2026-08-21）。IgH 是 Linux 2.6 及以上平台的内核态主站；核心模块在 `master/`，对应用提供 `include/ecrt.h` 与 `lib/` 用户态库。

| IgH 子系统 | 主要文件/目录 | 职责 | ESOP 处理 |
| --- | --- | --- | --- |
| 内核主站 | `master/master.*`、`module.c` | 主站生命周期、队列、统计、DC、设备集合、调度 | 保留生命周期与队列思想；剔除 Linux 内核对象和模块入口 |
| EtherCAT 数据报/帧 | `datagram.*`、`datagram_pair.*`、`ethernet.*` | 数据报元数据、帧组包解析、冗余设备帧配对 | 必需；首版实现单端口与限定帧队列 |
| 设备抽象 | `device.*`、`devices/ecdev.h` | 设备 offer/open/close、收帧、链路事件 | 必需；改成不包含 `net_device` 的平台无关 vtable |
| Domain 与 FMMU | `domain.*`、`fmmu_config.*`、`sync_config.*` | PDO 过程映像、逻辑地址、FMMU 和 Sync Manager 配置、WKC | 必需；这是零拷贝和多速率分组的核心模型 |
| 从站与配置 | `slave.*`、`slave_config.*`、`sync.*`、`pdo.*`、`pdo_entry.*` | 拓扑从站模型、SM/PDO/FMMU/DC 配置 | 必需；以静态描述和生成代码支持，不要求完整 Linux 风格对象树 |
| 主站/从站 FSM | `fsm_master.*`、`fsm_slave*`、`fsm_change.*`、`fsm_sii.*` | 扫描、状态切换、SII、从站配置与监控 | 必需；做成单步推进、可预算执行的 FSM |
| 邮箱与协议 FSM | `mailbox.*`、`fsm_coe.*`、`fsm_foe.*`、`fsm_soe.*`、`fsm_eoe.*` | 邮箱封装与协议特定状态机 | 邮箱框架与 CoE 必需；其他协议通过插件注册 |
| 异步请求 | `sdo_request.*`、`soe_request.*`、`reg_request.*`、`voe_handler.*` | 非周期 SDO/SoE/寄存器/厂商协议请求 | 必需的设计模式；首版仅 SDO 与寄存器请求 |
| 应用 API | `include/ecrt.h`、`lib/` | 申请主站、配置从站、创建 Domain、周期收发、DC、状态查询 | 必需；表面 API 保持更小，不兼容 `ecrt_*` ABI |
| 运维接口 | `cdev.*`、`ioctl.*`、`tool/`、`script/` | 字符设备、ioctl、命令行工具、systemd/sysconfig | 不进入嵌入式主站；后续以串口/RTT/UDP 诊断适配实现 |
| Linux 实时接口 | `rtdm.*`、`rtdm_xenomai_v3.c`、RTAI/Xenomai 示例 | 内核/用户态实时环境对接 | 不进入核心；由平台调度器决定 |
| 网络附加功能 | `eoe_request.*`、`tty/`、虚拟接口 | EoE 网络、虚拟 TTY、抓包接口 | 后续可选，不能影响周期任务 |
| 仿真和示例 | `fake_lib/`、`examples/` | 用户 API 假实现、不同实时环境示例 | 借鉴为 PCAP 回放与虚拟 ESC 测试，但独立实现 |

IgH 的关键架构价值是：Domain 将 PDO 映像与应用内存直接关联；主站、从站和邮箱事务用有限状态机推进；数据报有队列和固定环；实时应用显式调用 receive/process/queue/send。它的复杂度来自 Linux 内核、网卡驱动维护、字符设备、ioctl、RTDM、完整 CLI 及 EoE/TTY 生态，这些均与 MCU 主站的最小依赖目标冲突。

## 3. 产品边界

### 3.1 必须支持的首发范围

1. 单个 EtherCAT 网段、一个活动主站端口、线型或树型从站拓扑。
2. 至少 32 个从站的静态上限可配置；不因扫描结果使用堆分配。
3. EtherCAT 二层帧构造与解析，含常用物理和逻辑数据报。
4. 扫描、自动递增寻址、固定站地址分配、SII 基本读取、ESC 寄存器访问。
5. AL 状态机：INIT、PREOP、SAFEOP、OP、错误确认和状态读取。
6. Sync Manager、FMMU、静态 PDO 映射与逻辑过程映像（Domain）。
7. PDO 周期收发、WKC 计算与连续异常检测。
8. CoE 邮箱中的 SDO expedited 和 segmented 上传/下载；启动阶段的 PDO 分配/映射写入。
9. DC 参考时钟选择、应用时间输入、SYNC0 配置、参考时钟与从站时钟同步。
10. CiA 402 驱动状态机、CSP/CSV/CST、Controlword/Statusword 和驱动故障处理，作为独立 profile 模块。
11. Mailbox Resilient Layer、输入邮箱轮询、CoE Emergency 接收及从站到从站数据复制。
12. 链路、帧、WKC、AL、邮箱 abort、DC 偏差的结构化诊断与事件。
13. STM32、HPMicro 和 Linux 用户态各一个可构建、可回归验证的端口。其中 Linux 端口用于开发和硬件在环，不是核心运行前提。

### 3.2 明确不进入首发范围

- Linux 内核模块、私有网卡驱动 fork、字符设备、ioctl、RTDM、systemd 服务和 CLI；
- 双网口冗余、热备主站、多主站；
- EoE、FoE、SoE、AoE、VoE、虚拟 TTY、完整对象字典浏览；
- 运行时解析大型 ENI/XML。ENI 到 C 配置描述的转换属于宿主工具，不进入 MCU 固件；
- 自动重拓扑后无条件回到 OP。恢复动作必须由应用策略确认；
- 宣称 EtherCAT 一致性/认证，除非完成适用的外部一致性流程。

### 3.3 硬件准入条件

平台端口只有满足以下条件才可标记为“支持”：

| 条件 | 要求 |
| --- | --- |
| MAC/PHY | 100 Mbit/s 全双工 Ethernet MAC 与正确 PHY 时钟、RMII/MII 配置；链路状态可读取 |
| 二层访问 | 能收发 EtherType `0x88A4` 的原始以太网帧，不经过 TCP/IP、ARP 或普通 socket 缓冲路径 |
| DMA | 支持固定 TX/RX descriptor 和调用方提供的 DMA 对齐缓冲；RX 帧能在确定的预算内被轮询/通知 |
| 缓存一致性 | 有 D-cache 的 SoC 必须由端口在所有权切换点执行 clean/invalidate，并把 descriptor/缓冲放入合适的内存区 |
| 时间基准 | 提供单调纳秒或至少 32 位微秒扩展时基；DC 模式建议有稳定的高分辨率定时器 |
| 中断 | ISR 仅确认 DMA/记录事件/唤醒任务；不得在 ISR 中运行扫描、邮箱或应用回调 |
| 资源 | 能预留按配置计算的过程映像、帧槽、邮箱槽和从站表；RAM/Flash 数值由构建报告给出 |

## 4. 架构

### 4.1 分层

```text
Application / Motion / PLC task
  |  cycle(), service(), event callback, process image
  v
Public API + static configuration descriptors
  |
  +-- Control plane: scan -> SII -> AL -> SM/FMMU -> CoE config -> DC
  |       |             (budgeted, non-blocking state machines)
  |
  +-- Data plane: Domain -> datagram planner -> frame builder/parser -> WKC
  |       |             (allocation-free, bounded work per cycle)
  |
  +-- Diagnostics: counters, events, fixed trace ring
  v
Port interface: time / critical section / DMA cache / raw Ethernet MAC
  v
STM32 HAL/LL | HPM SDK | Linux AF_PACKET test port | other BSP
```

### 4.2 模块和依赖规则

| 模块 | 责任 | 可依赖 | 不可依赖 |
| --- | --- | --- | --- |
| `ecm_wire` | wire-endian、EtherCAT/数据报头序列化、边界校验 | C 标准头 | OS、端口、配置、堆 |
| `ecm_engine` | 帧槽、数据报队列、帧构建、匹配、超时、WKC | `wire`、`port` | 邮箱协议、应用、平台 SDK |
| `ecm_domain` | PDO 条目、逻辑地址、过程映像、FMMU/SM 计划 | `wire`、`engine` | 动态分配、平台 SDK |
| `ecm_scan` | 探测、地址分配、SII、拓扑快照 | `engine`、`al` | 应用任务、字符串格式化 |
| `ecm_al` | ESC 访问、AL 状态和状态机 | `engine` | 线程、阻塞睡眠 |
| `ecm_dc` | 拓扑延迟、参考钟、DC 同步和 SYNC 配置 | `engine`、`port.time_now_ns` | 平台专有计时逻辑 |
| `ecm_mbox` | 邮箱发送/接收、计数器、请求生命周期 | `engine`、`al` | 周期数据面锁 |
| `ecm_coe` | CoE/SDO、PDO 配置状态机 | `mbox` | 其他邮箱协议实现 |
| `ecm_diag` | 错误码、计数器、事件、追踪 ring | 基础类型 | stdout、文件系统、堆 |
| `ecm_port` | 原始 MAC、DMA、缓存、临界区、时钟 | BSP/RTOS | 协议模块 |
| `ecm_cfggen` | ENI/ESI/JSON 到静态 C 描述的宿主工具 | 宿主库可选 | 固件核心 |

核心模块间只能单向依赖。新增邮箱协议必须实现 `ecm_mbox_protocol_ops`，新增平台必须实现 `ecm_port_ops`；两类扩展都不得修改 `ecm_engine` 的公共数据结构。

### 4.3 执行模型

主站不创建线程，也不在内部休眠。调用方负责把任务绑定至最高优先级定时器、RTOS 线程或 Linux 实时线程。

```c
/* 示例 API，签名可在设计阶段细化，生命周期与时序必须保持。 */
ecm_init(&master, &static_config, memory, &port_ops, port_context);
ecm_start(&master);                 /* 非阻塞，进入扫描/配置 FSM */

for (;;) {
    uint64_t now = app_time_ns();
    ecm_service(&master, now, control_plane_budget_us);

    if (cycle_due(now)) {
        ecm_domain_before_tx(domain);    /* 应用已写 outputs */
        ecm_cycle_send(&master, now);    /* 只提交准备好的 PDO/DC 帧 */
        ecm_cycle_receive(&master, now); /* 匹配 RX、更新 inputs/WKC */
        ecm_domain_after_rx(domain);
    }
}
```

约束：`ecm_service`、`ecm_cycle_send`、`ecm_cycle_receive` 都有明确上限工作量；任何邮箱、SII、状态转换或超时必须跨多次调用推进。应用不得同时从两个上下文调用同一个 `ecm_master_t`，除非使用由端口提供的外部串行化。

## 5. 可验证需求

优先级说明：P0 是首版发布阻塞项；P1 是首版后立即实现的兼容能力；P2 是按产品需求接入的插件。

### 5.1 核心与移植

| ID | 优先级 | 需求 | 验收证据 |
| --- | --- | --- | --- |
| ARC-001 | P0 | 协议核心采用 Rust `no_std`，禁止 POSIX、Linux、RTOS 和平台 SDK 依赖。 | `crates/esop-ethercat-core/` 在 host、Cortex-M 和 RV32 交叉编译中通过；依赖图无违规。 |
| ARC-002 | P0 | 核心仅通过 `ecm_port_ops` 访问原始收发、时间、临界区和 DMA 缓存。 | 替换 Linux/STM32/HPM 三个端口无需修改 `core/`。 |
| ARC-003 | P0 | `init` 后，周期 API、FSM 推进和错误记录不得调用 malloc/free/new。 | 链接包装或静态分析证明运行期 0 次分配；压力测试通过。 |
| ARC-004 | P0 | 所有容量由 `ecm_limits_t` 和调用方提供的内存块决定；初始化失败必须报告所需字节数。 | 32 从站基线与超限配置测试均通过，无越界。 |
| ARC-005 | P0 | 端口必须定义 TX/RX 所有权、cache 维护、descriptor 对齐、链路状态和帧最大长度。 | 平台端口契约测试和 DMA cache 压力测试通过。 |
| ARC-006 | P1 | 端口可选提供硬件 TX/RX 时间戳，DC 模块可利用但不强制依赖。 | 模拟时间戳端口测试通过。 |

### 5.2 帧与数据报引擎

| ID | 优先级 | 需求 | 验收证据 |
| --- | --- | --- | --- |
| ENG-001 | P0 | 支持 APRD/APWR/APRW、FPRD/FPWR/FPRW、BRD/BWR/BRW、LRD/LWR/LRW、ARMW、FRMW 的编码、解析和 WKC 提取。 | 每种数据报有 golden-frame 单元测试、短帧/长度溢出/follow 位异常测试。 |
| ENG-002 | P0 | 一个 Ethernet 帧可打包多个 EtherCAT 数据报；计划器按 MTU、领域和优先级拆帧。 | 1、N、超过单帧上限三类测试验证顺序、长度和 WKC。 |
| ENG-003 | P0 | 每个待发送帧使用固定帧槽、唯一索引和截止时间；迟到、重复、未知 RX 帧不能破坏过程映像。 | 乱序/重复/超时 PCAP 回放测试通过。 |
| ENG-004 | P0 | 周期数据面禁止忙等收包；`cycle_receive` 仅在可配置时间预算内轮询。 | 用模拟未回帧场景测得函数时长不超过预算。 |
| ENG-005 | P1 | 每帧和每领域分别统计 expected/actual WKC、timeout、unmatched、corrupt。 | 诊断 API 和故障注入测试通过。 |
| ENG-006 | P1 | 首版仅一个活动端口；端口数必须是配置参数而非散落编译条件。 | 单端口产品构建不含冗余状态；双端口原型可在不改 core API 下接入。 |

### 5.3 扫描、ESC、AL 和配置

| ID | 优先级 | 需求 | 验收证据 |
| --- | --- | --- | --- |
| CFG-001 | P0 | 上电扫描以自动递增寻址发现从站，读取基本 ESC 信息并分配固定站地址。 | 1、8、32 从站 HIL 拓扑报告与地址唯一性测试。 |
| CFG-002 | P0 | 支持 ESC 寄存器读写和 SII 基础读取，输出 vendor ID、product code、revision、serial、mailbox/SM/FMMU 能力及端口拓扑。 | 已知从站数据与 SII 交叉验证。 |
| CFG-003 | P0 | 实现 INIT/PREOP/SAFEOP/OP 状态转换、错误确认和状态超时；失败事件必须带从站号、请求/实际状态、AL status code。 | 每条转换及错误状态的故障注入测试。 |
| CFG-004 | P0 | 静态配置必须按 alias/position 和 vendor/product/revision 匹配从站；不匹配时禁止进入 OP。 | 正常、型号错误、位置错误三组测试。 |
| CFG-005 | P0 | 支持 SM、FMMU、watchdog 和固定逻辑地址的配置；逻辑映像必须可复现。 | 对同一配置多次启动得到相同映射和帧计划。 |
| CFG-006 | P0 | 支持程序化静态 PDO 配置以及由 `ecm_cfggen` 生成的等价 C 描述。 | 两种输入生成相同 SM/FMMU/PDO 写序列。 |
| CFG-007 | P1 | 支持配置阶段读取完整 SII PDO/SM 类别，作为静态描述的校验来源。 | ESI/配置/实际 SII 比对报告。 |
| CFG-008 | P0 | 识别并正确处理带/不带 Device Emulation 的从站；不得对 Device Emulation 从站错误使用 AL Error Acknowledge。 | 两类虚拟 ESC 的状态切换与错误确认序列测试。 |
| CFG-009 | P0 | ESM 转换使用 ESI/SII 提供的超时；缺失时使用受版本管理的 ETG.1020 默认值。`OpOnly` 设备在非 OP 状态必须禁用输出 SyncManager。 | 超时覆盖、默认回退和 `OpOnly` 输出隔离 HIL。 |
| CFG-010 | P1 | 支持 Explicit Device Identification，并可按配置用于防止换线/错位设备进入 OP。 | 交换两个同型号设备或修改 Identification ADO 后拒绝激活。 |

### 5.4 Domain 与周期 PDO

| ID | 优先级 | 需求 | 验收证据 |
| --- | --- | --- | --- |
| PDO-001 | P0 | Domain 可注册 RxPDO/TxPDO 条目，返回 bit offset、bit length 和字节访问辅助方法。 | 位域跨字节、8/16/32/64 位及 signed/unsigned 测试。 |
| PDO-002 | P0 | 过程映像由调用方提供，应用可直接读写，周期路径不额外复制整个映像。 | 指针一致性和 cycle allocation/copy 统计测试。 |
| PDO-003 | P0 | 每个 Domain 生成稳定的逻辑地址和预期 WKC；允许一个主站有多个 Domain。 | 两个不同更新率 Domain 的帧计划和 WKC 测试。 |
| PDO-004 | P0 | `cycle_send` 仅发送已激活 Domain，`cycle_receive` 仅在匹配帧完整且 WKC 合格时提交输入。 | 丢帧、半帧、WKC 失配时输入保留旧值或标记无效。 |
| PDO-005 | P1 | 对每个 Domain 公开有效性、最后成功周期、连续 WKC 失配数和输入年龄。 | 故障注入后状态转换正确。 |
| PDO-006 | P0 | 支持由静态配置描述的 Slave-to-Slave communication via master，在不超过两周期的有界路径中复制并携带源数据质量。 | 源从站到目标从站复制、源 WKC 错误和数据过期测试。 |

当前实现已增加 `DomainRegistry`：它在激活前以固定容量登记多个 Domain、PDO entry 和 datagram，自动返回稳定的 Domain-local bit offset，并把 datagram 的相对过程映像 offset 转换为全局 `FramePlan` offset。注册表同时校验 Domain 过程映像/逻辑地址重叠、全局 datagram index、PDO bit overlap、WKC 溢出和多速率 hyperperiod；`activate` 成功后拒绝继续注册。`SiiConfigurationCandidate` 可冻结为 `SiiDomainProjection`，在进入注册表前再次核对 Rx/Tx 统一映像偏移、FMMU 与 SyncManager 物理范围、逻辑基址和映像容量；字节对齐 segment 可由 `LWR`/`LRD` 自动绑定，位打包 segment 必须由调用方提供聚合 datagram。`FramePlanSet` 在激活期按 MTU 和固定容量拆帧，计划与 phase 采用原子发布；失败不会发布部分 Domain/PDO/计划。现有 `Domain` 的输入 staging 段和 `ScheduleTable` 可由注册结果直接生成。真实从站的 PDO 互操作、FMMU/SM read-back 与硬件 HIL 仍需完成。

### 5.5 DC 与时间

| ID | 优先级 | 需求 | 验收证据 |
| --- | --- | --- | --- |
| DC-001 | P0 | 扫描阶段识别 DC 能力并建立从站传播延迟/参考时钟信息；无 DC 从站时功能可关闭且不影响 PDO。 | DC 和非 DC 拓扑分别通过启动测试。 |
| DC-002 | P0 | 应用以单调纳秒时间提供 `application_time`；主站支持参考时钟同步、从站时钟同步和 SYNC0 周期/相位配置。 | 具备 DC 的驱动从站上采集偏差、SYNC0 周期一致性。 |
| DC-003 | P1 | 公开 DC 偏差、最后同步时间、失锁状态；连续失锁不得被静默忽略。 | 时钟跳变/延迟注入测试。 |
| DC-004 | P1 | DC 同步数据报与 PDO 数据报可共帧或分帧，策略由帧计划器显式选择。 | 两种计划都能通过 WKC 与周期测试。 |

### 5.6 邮箱、CoE 和扩展协议

| ID | 优先级 | 需求 | 验收证据 |
| --- | --- | --- | --- |
| MBX-001 | P0 | 邮箱收发、计数器、重复帧、超时、错误响应必须由非阻塞请求状态机处理。 | 请求跨多个 service 调用完成，周期路径无阻塞。 |
| COE-001 | P0 | 支持 CoE SDO expedited 和 segmented upload/download，报告 abort code。 | 标准对象读写、分段传输、abort 和超时测试。 |
| COE-002 | P0 | 可在 PREOP/SAFEOP 配置阶段执行 SDO 写入以设置 PDO assignment/mapping。 | 已知 CoE 从站启动后 PDO 映射与期望一致。 |
| MBX-002 | P1 | 支持 Complete Access 的 SDO 传输，但必须允许设备配置禁用。 | CA 支持/拒绝两类设备的兼容测试。 |
| MBX-003 | P0 | 实现与上层邮箱协议无关的 Mailbox Resilient Layer，恢复丢失、重复或状态不一致的邮箱帧。 | 丢帧、重复帧、计数器回绕和重试故障注入。 |
| MBX-004 | P0 | 按 ESI/SII/静态配置支持周期轮询输入邮箱或映射 Mailbox Status Bit；轮询严格受控制面预算限制。 | PollTime 与 StatusBit 两种设备模型 HIL。 |
| COE-003 | P0 | 接收 CoE Emergency 消息并以固定事件记录交给应用；应用未及时消费不得阻塞邮箱 FSM。 | 多驱动并发 Emergency、事件环满和顺序测试。 |
| COE-004 | P1 | 支持 SDO Information service，用于读取对象、类型、访问权和 PDO 可映射属性；可在产品构建中关闭。 | 与已知对象字典及 ESI 描述交叉验证。 |
| REG-001 | P1 | 提供异步 ESC 寄存器请求 API，仅在控制面预算中执行。 | 寄存器读写不会推迟指定 PDO 周期。 |
| EXT-001 | P2 | FoE、SoE、EoE、VoE 必须作为独立协议模块注册，不得改动数据报核心。 | 编译开关和插件 API 回归测试。 |

### 5.7 CiA 402 与运动控制 Feature Pack

| ID | 优先级 | 需求 | 验收证据 |
| --- | --- | --- | --- |
| CIA-001 | P0 | `esop_profile_cia402` 实现 CiA 402 PDS FSA，依据 Statusword 判定状态并生成 Shutdown、Switch on、Enable operation、Quick stop、Disable voltage 和 Fault reset 命令。 | 全状态/转换表单元测试及两种厂商驱动 HIL。 |
| CIA-002 | P0 | 支持 `0x6040`、`0x6041`、`0x6060`、`0x6061`、`0x603F`，并按所选模式绑定目标值、实际值和限制对象；对象存在性与访问权来自 ESI/SDO/设备能力。 | 对象缺失、只读、值拒绝及 read-back 测试。 |
| CIA-003 | P0 | 机器人基线支持 CSP、CSV、CST；模式切换必须经过配置的停机/状态序列并确认 Modes display，禁止仅写 mode 后继续运动。 | 三模式启动、在线切换、拒绝切换和超时 HIL。 |
| CIA-004 | P0 | CiA 402 profile 与 DC 联合验收；DC 未锁定、WKC 无效、命令过期或驱动状态异常时不得发布新的有效运动目标。 | DC 失锁、WKC 故障、命令超时和驱动 fault 注入。 |
| CIA-005 | P1 | Homing、Profile Position/Velocity/Torque 和厂商对象通过 capability/quirk 描述扩展，不得把厂商偏差写入 EtherCAT core。 | 两厂商 profile 描述产生不同配置但共用相同 core。 |
| CIA-006 | P0 | 配置与诊断记录 CiA 402/IEC/ETG.6010 基线版本；不得假定所有驱动实现相同可选对象或全部模式。 | 配置 manifest、能力协商和不兼容报告审查。 |

### 5.8 诊断、故障与安全行为

| ID | 优先级 | 需求 | 验收证据 |
| --- | --- | --- | --- |
| DIA-001 | P0 | 固定容量事件环至少记录时间、严重级别、模块、从站、帧/请求 ID、错误码和上下文值。 | 溢出时保留丢失计数且不破坏周期通信。 |
| DIA-002 | P0 | 公开 master/link/domain/slave 状态快照；查询 API 不得分配内存或格式化日志。 | 并发快照与压力测试。 |
| DIA-003 | P0 | 连续 WKC 异常、帧超时、链路断开和 AL 错误必须触发事件；默认动作是保持过程映像有效性标志为失败而不是自动写输出。 | 四种故障注入和应用策略测试。 |
| REC-001 | P1 | 提供显式 `rescan`、`reconfigure_slave`、`request_state`，这些操作只在控制面执行。 | 周期运行中恢复测试，证明周期预算不被破坏。 |
| REC-002 | P1 | 自动恢复必须是应用配置的策略，默认关闭；策略可规定安全输出、重试次数和是否允许回到 OP。 | 策略矩阵测试。 |
| SAF-001 | P0 | 主站不替代功能安全系统；任何通信异常的安全输出和 STO 行为由应用/设备安全链路负责。 | API/文档审查无错误安全承诺。 |

### 5.9 能力声明与一致性边界

| ID | 优先级 | 需求 | 验收证据 |
| --- | --- | --- | --- |
| CON-001 | P0 | 每个发布物声明 ETG.1500 基本功能集版本、Feature Pack 版本、支持/不支持功能和平台限制。 | 机器可读 capability manifest 与发布说明一致。 |
| CON-002 | P0 | 完成内部测试只允许声明“按 Class B + Motion FP 设计/测试”；只有完成适用的 ETG 官方一致性流程后才能使用认证或正式 conformance 表述。 | 发布文案检查和外部测试报告归档。 |

## 6. 性能与资源目标

性能指标必须在“硬件 + PHY + 从站拓扑 + 帧负载 + RTOS 配置”组合上验证，不能用单一 MCU 数字代表全部平台。以下是首版的合格基线，不是以太网线缆延迟的承诺。

| ID | 目标 | 基线与测量方法 |
| --- | --- | --- |
| PERF-001 | 无运行期分配 | `init` 成功后运行 10 分钟周期/邮箱混合负载，malloc/free/new 调用为 0。 |
| PERF-002 | 有界周期路径 | 在 32 从站、单 Domain、总 PDO 负载不超过 512 B、DC 开启、500 us 周期的合格硬件上，`cycle_send + cycle_receive` 的软件 P99 用时不超过周期预算的 25%。记录 CPU 时钟与测量点。 |
| PERF-003 | 250 us 能力 | 在 16 从站、总 PDO 负载不超过 256 B、单帧、DC 可选的合格硬件上，连续 30 分钟完成 250 us 周期；报告最大 jitter、timeout 和 WKC 错误数。 |
| PERF-004 | 控制面隔离 | 执行 SDO/扫描/诊断时，PDO 周期 P99 jitter 相对于空载基线的增加不超过 10%，否则降低控制面预算或拒绝请求。 |
| PERF-005 | 默认内存上限 | 32 从站、1 KiB 过程映像、8 帧槽、8 邮箱请求、256 条追踪事件的核心状态（不含 DMA descriptor/过程映像/平台 SDK）目标小于 64 KiB RAM；构建产物必须输出实际值。 |
| PERF-006 | 代码体积 | 关闭可选协议、开启 CoE+DC 的 Cortex-M release 构建目标小于 128 KiB `.text + .rodata`；实际结果随编译器记录。 |

PERF-002 和 PERF-003 的“合格硬件”必须是通过第 3.3 节准入测试的具体开发板和 PHY，且报告使用的从站型号、线缆长度、RTOS tick/中断优先级、编译器和优化等级。若某 STM32 型号或板卡未达到目标，应降低其支持等级而不是降低通用指标或在核心中添加平台特例。

## 7. 配置、内存和并发约束

1. `ecm_master_t`、从站表、Domain 表、帧槽、邮箱请求池、事件环和过程映像均由调用方或生成配置提供；不允许核心拥有隐藏静态单例。
2. 每个对象有明确生命周期：`uninitialized -> initialized -> configuring -> safeop -> operational -> stopping -> stopped/fault`。非法调用返回错误，不做隐式状态跳转。
3. 过程映像的 output 只能由应用写入；input 只在成功匹配的 RX 提交点更新。对有并发应用任务的平台，应用使用自身的双缓冲/临界区，主站不隐藏锁。
4. DMA 缓冲要满足端口声明的对齐、地址范围和 cache 属性；端口不可假定普通 `malloc` 内存可 DMA。
5. 线缆字节序读写必须经过 `ecm_get_leXX`/`ecm_put_leXX` 类接口，不允许将 packed wire struct 直接解引用。
6. 错误码为稳定枚举；可选字符串转换只在诊断编译选项启用，且不可从 ISR 或周期 API 调用。

## 8. 平台端口要求

### 8.1 统一端口接口

端口的最小职责：

```c
typedef struct {
    int      (*open)(void *ctx, const ecm_port_config_t *cfg);
    void     (*close)(void *ctx);
    int      (*tx_acquire)(void *ctx, ecm_dma_buf_t *buf);
    int      (*tx_submit)(void *ctx, ecm_dma_buf_t *buf, size_t length);
    unsigned (*rx_poll)(void *ctx, ecm_rx_frame_t *out, unsigned max_frames);
    void     (*rx_release)(void *ctx, const ecm_rx_frame_t *frame);
    bool     (*link_up)(void *ctx);
    uint64_t (*time_now_ns)(void *ctx);
    void     (*dma_prepare_tx)(void *ctx, const void *addr, size_t length);
    void     (*dma_complete_rx)(void *ctx, void *addr, size_t length);
    ecm_irq_state_t (*critical_enter)(void *ctx);
    void     (*critical_exit)(void *ctx, ecm_irq_state_t state);
} ecm_port_ops_t;
```

`tx_acquire` 和 `rx_poll` 的返回值必须区分“暂时无 descriptor”“链路断开”“DMA 错误”和“帧可用”。MAC 驱动可采用中断唤醒 + 轮询 drain，但回调不得从硬中断上下文进入核心。

### 8.2 首批端口

| 端口 | 状态要求 | 注意事项 |
| --- | --- | --- |
| `port_linux_raw` | P0 开发/CI/HIL 端口 | 仅做原始二层收发和单调时钟；不得成为核心 API 的依赖。可用于 PCAP 回放与实物从站联调。 |
| `port_stm32_eth` | P0 参考 MCU 端口 | 支持至少一个带 100M Ethernet MAC 的确定型号/评估板；明确 HAL 或 LL 版本、RMII 引脚、MPU/D-cache 和 DMA 内存段。 |
| `port_hpm_gmac` | P0 参考 MCU 端口 | 支持至少一个 HPMicro 确定型号/评估板；按其 RISC-V toolchain、GMAC/DMA 和 cache 属性实现。 |
| `port_freertos` | P1 | 只提供任务/中断集成示例，不将 FreeRTOS 类型暴露给核心。 |
| `port_zephyr` | P1 | 复用同一 `port_ops`，以独立适配层接入。 |

每个端口目录必须提供：板级配置、MDIO PHY 链路检查、TX/RX descriptor 所有权说明、cache 操作说明、最小 loopback/帧注入测试、一个实际 EtherCAT 从站 HIL 测试脚本及已知限制。

## 9. 交付分期

| 里程碑 | 可交付范围 | 发布门槛 |
| --- | --- | --- |
| M0：Wire + Port | `wire`、固定帧槽、Linux raw 仿真端口、通用 DMA descriptor ownership/cache 契约、Frame Plan 到 DMA TX descriptor 的直接构建、DMA TX 提交端口边界、DMA RX descriptor 直接消费会话、STM32/HPM MAC bring-up | 所有数据报编码/解析单测；可接收/发送 EtherType `0x88A4`；通用描述符状态机、缓存维护、TX 零中间帧拷贝构建、RX descriptor 直接解析和提交失败回收测试通过，目标板 DMA 缓存测试仍需实板完成。 |
| M1：最小 PDO 主站 | scan、固定地址、AL、SM/FMMU 写入读回、单 Domain、LRW、WKC、诊断 | 1/8/32 从站 HIL，达到 SAFEOP/OP，过程数据连续 1 小时无内存增长。 |
| M2：可用驱动主站 | CoE SDO、PDO 配置、DC、多个 Domain、恢复 API | 已具备固定容量多 Domain 注册、SII segment datagram 绑定、MTU 拆帧和多速率激活编排；仍需具备 CoE/DC 驱动从站的 250/500 us 基线、故障注入和 jitter 报告。 |
| M3：产品化扩展 | 配置生成器、Complete Access、寄存器请求、RTOS 示例、PCAP 回放 | 配置可复现、跨端口回归、资源报告和 API 兼容性检查。 |
| M4：可选协议 | FoE、SoE、EoE、VoE、冗余 | 每个协议独立开关、独立测试和对周期性能影响报告。 |

M1 是“完成必需 EtherCAT PDO 功能”的最小可用版本；M2 是“适合带 DC 伺服设备的实际控制”的目标版本。不得为了赶 M1 把同步等待或无边界扫描逻辑放入周期 API。

## 10. 测试与发布标准

### 10.1 自动测试

1. Host 单元测试：所有 wire 编码、数据报组包/解析、WKC、位域 PDO、超时、FSM 状态转移。
2. 属性/模糊测试：长度字段、follow 位、错误索引、重复帧、截断邮箱和 SDO abort。
3. PCAP 回放：正常、乱序、丢失、WKC 错误、拓扑变化和 mailbox 分段序列。
4. 资源测试：全功能配置在 host/ARM/RV32 三个目标运行静态大小和 stack 分析；周期期间分配拦截为零。
5. 端口契约测试：RX/TX descriptor 耗尽、DMA 错误、PHY 断链、cache 维护顺序。

### 10.2 硬件在环

| 场景 | 必须验证 |
| --- | --- |
| 最小链路 | 主站 + 1 个 CoE 从站：扫描、PREOP/SAFEOP/OP、SDO、PDO、AL 错误。 |
| 规模链路 | 8 和 32 从站：地址、拓扑、Domain、WKC 和连续周期。 |
| DC 链路 | 至少一个 DC 驱动从站：参考钟、SYNC0、偏差和失锁恢复。 |
| 故障 | 拔插、从站掉电、错误 AL 状态、WKC 失配、邮箱 abort、RX descriptor 耗尽。 |
| 平台 | Linux raw、一个 STM32 板、一个 HPMicro 板均执行同一基本用例。 |

### 10.3 发布阻塞条件

- P0 需求全部有自动化或 HIL 证据；
- 目标板在声明的负载和周期下无未解释 timeout、WKC 错误、DMA 错误或内存增长；
- 未支持的从站能力、芯片/板卡、PHY 和配置限制在发布说明中明确列出；
- 不使用 SOEM 或 IgH 源文件、复制代码或受其许可证约束的派生实现；
- 不以“兼容 EtherCAT”或“已认证”进行超出测试证据的宣传。

## 11. 许可证与合规边界

SOEM 官方仓库声明 GPLv3 与商业许可证双许可；IgH 内核主站为 GPLv2，用户态库/API 为 LGPL-2.1。若 ESOP 需要闭源商用、宽松开源许可或独立许可证，不能通过复制、翻译或改写上述实现获得。可研究功能边界和公开协议行为，但实现需独立编写，并由项目负责人确认 EtherCAT 技术、商标、测试和分发义务。

本节不是法律意见。发布前应完成许可证扫描、第三方清单、代码来源审计以及适用的 EtherCAT 技术/一致性咨询。

## 12. 源码依据与可复现性

分析只以官方项目源码和其维护的文档入口为依据，快照在 2026-09-02 获取：

1. SOEM，提交 `2f73eaa803f91f8332b5c8b047ba03a1210c9a80`：
   - `README.md`、`LICENSE.md`、`CMakeLists.txt`
   - `src/ec_base.c`、`ec_main.c`、`ec_config.c`、`ec_dc.c`、`ec_coe.c`、`ec_foe.c`、`ec_eoe.c`、`ec_soe.c`
   - `include/soem/ec_*.h`、`osal/`、`oshw/`
   - https://github.com/OpenEtherCATsociety/SOEM/tree/2f73eaa803f91f8332b5c8b047ba03a1210c9a80
2. IgH EtherCAT Master，提交 `650888c587b1aa570c4d7211adaf39145d9e5ae3`：
   - `README.md`、`FEATURES.md`、`include/ecrt.h`
   - `master/`、`devices/ecdev.h`、`lib/`、`tool/`、`tty/`、`fake_lib/`
   - https://gitlab.com/etherlab.org/ethercat/-/tree/650888c587b1aa570c4d7211adaf39145d9e5ae3
3. IgH 维护的 API 文档入口：
   - https://docs.etherlab.org/ethercat/1.6/doxygen/index.html

后续实现开始前，应把上述提交 hash 和本需求文档版本写入 `THIRD_PARTY_NOTICES.md` 与架构决策记录，确保代码审计可追溯。
