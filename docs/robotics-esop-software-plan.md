# ESOP 机器人软件规划

- 文档版本：0.1
- 日期：2026-09-03
- 状态：架构基线，供机器人产品、主站、平台和 ROS 2 集成共同使用
- 相关文档：[EtherCAT 主站需求与架构说明](ethercat-master-requirements.md)

## 1. 定义与结论

`ESOP` 是 **EtherCAT Simple Operating System**。在机器人产品中，它不是通用用途 OS，也不尝试取代 Linux、FreeRTOS、Zephyr 或 ROS 2；它是由 EtherCAT 实时数据面、设备抽象、过程数据缓冲和机器人集成服务构成的控制运行时。

本文将用户所写的 `proctbuf` 按 **Protocol Buffers（protobuf）** 理解。如果该词实际指另一个内部协议，必须在 R0 里修改术语和契约；本规划同时定义了名为 **ProcBuf** 的过程缓冲，避免把两者混淆：

| 名称 | 用途 | 是否处于 EtherCAT 周期路径 |
| --- | --- | --- |
| `ProcBuf` | 固定布局、调用方提供内存的机器人实时过程数据缓冲 | 是 |
| `protobuf` | 跨进程、跨设备的结构化命令、状态、事件、配置和记录数据契约 | 否 |
| ROS 2 `rosidl`/CDR | ROS 2 topic、service、action 的类型与序列化 | 否 |

**核心决策：禁止把 Protobuf、Zenoh、ROS 2 或动态配置解析放入 PDO 的确定性收发循环。**EtherCAT PDO 必须使用固定长度的过程映像和确定的拷贝/交换次数。Protobuf 的优点是跨语言、跨平台和可演进的结构化序列化；它适合控制面和观测面，而非 500 us 至 2 ms 的伺服数据面。[S1]

ROS 2 通过 `rmw` 抽象底层通信；Zenoh 可作为 ROS 2 的 RMW 实现之一。因此 ESOP 不实现新的 RMW：ROS 2 侧使用已有 `rmw_zenoh_cpp` 或 DDS RMW，ESOP 只提供硬件接口和独立 Zenoh 网关。[S2][S3]

## 2. 机器人使用 EtherCAT 的功能范围

EtherCAT 在机器人中负责**确定性执行和采集**，而不是把所有机器人计算都放到现场总线上。

| 机器人能力 | EtherCAT 功能 | ESOP 首版责任 | 周期建议 |
| --- | --- | --- | --- |
| 关节伺服 | CoE、CiA 402 profile、RxPDO/TxPDO、DC/SYNC0、CSP/CSV/CST 模式 | P0 | 500 us 或 1 ms；实际值由驱动、机构和 MCU 测试决定 |
| 移动底盘 | 多轴驱动、编码器、制动状态、IO | P0 | 1-2 ms |
| 夹爪/末端执行器 | 伺服、步进、数字 IO、压力/位置反馈 | P0 | 1-4 ms |
| 分布式 IO | DI/DO、AI/AO、温度、限位、急停链路状态 | P0 | 1-8 ms，按 Domain 分组 |
| 力/力矩、扭矩、编码器 | TxPDO 采样、DC 时间戳、质量状态 | P1 | 1-4 ms；高带宽原始数据不承诺走 EtherCAT |
| IMU、相机、雷达 | 仅状态/触发/同步 IO 可进 EtherCAT；主数据走专用接口 | P1 | 不进入 EtherCAT 主数据面 |
| 参数与诊断 | CoE SDO、对象字典、AL 状态、错误历史 | P0 | 非周期、异步、受预算限制 |
| 驱动固件升级 | FoE 或厂商协议 | P2 | 维护模式，不可与运动周期并行 |
| 功能安全 | FSoE 或独立认证安全链 | P2/产品专项 | 安全链路独立；ESOP 不自称安全主站 |

### 2.1 首版必须实现的 CoE 机器人能力

1. 扫描并验证从站身份：alias/position、vendor ID、product code、revision。
2. CoE SDO expedited 与 segmented upload/download；支持启动阶段的对象写入、读取回校验和 abort code 诊断。
3. 静态 PDO assignment/mapping，支持常见 CiA 402 驱动的 controlword、statusword、operation mode、位置/速度/转矩目标与实际值。
4. CiA 402 驱动状态机作为 ESOP profile 插件，而不是散落在应用代码中。
5. DC 参考钟、SYNC0、周期稳定性、WKC 和 input age 诊断。
6. 对运动使能、状态转换、PDO 质量、驱动错误恢复实行显式状态机；异常时默认禁止继续更新运动输出。

CiA 402 不是 EtherCAT 强制的唯一设备 profile。因此 `cia402` 只能作为首个 profile，不得把 ESOP 的设备模型绑定到某一驱动厂商或把所有 EtherCAT 从站假定为伺服驱动。

### 2.2 机器人运动控制边界

ESOP 负责把来自控制器的每轴命令在确定周期内写入驱动，并将编码器、驱动状态、WKC 和时钟质量返回。它可包含关节级的限幅、斜坡、通信看门狗和使能状态机。

运动学、轨迹规划、碰撞规划、视觉、SLAM、行为树和大模型推理运行在 ROS 2/Linux 侧。内环位置/速度/力矩控制可部署在 ESOP 的实时节点或驱动器中，但必须以具体关节带宽、传感器延迟和 CPU 利用率验证；不能仅凭 EtherCAT 周期宣称控制品质。

## 3. 总体部署架构

产品默认采用**双域部署**：实时节点负责 EtherCAT 和 ProcBuf，Linux 监督节点负责 ROS 2、Zenoh、记录、UI 和高层控制。开发期允许单机部署，但其实时性能不能替代目标 MCU 的测试结果。

```text
                Linux ARM/x86 supervision domain
 +-------------------------------------------------------------+
 | ROS 2 graph / ros2_control / planning / perception          |
 |       |                              |                       |
 | esop_ros2_control                esop_zenoh_gateway          |
 |       | ROS 2 CDR                 | Protobuf over Zenoh       |
 |       +---------- esop_ipc -------+---------------------------+
 +------------------------|------------------------------------+
                          | shared memory / RPMsg / UDS
 +------------------------v------------------------------------+
 | STM32/HPM/ARM real-time ESOP domain                          |
 | robot_runtime -> ProcBuf -> ecat scheduler -> raw Ethernet   |
 |        |             |              |                         |
 | periph adapters       +--> CoE / CiA402 / IO / DC             |
 +------------------------|------------------------------------+
                          | EtherType 0x88A4
                   EtherCAT drives, IO, sensors
```

| 部署模式 | 用途 | 实时节点 | ROS 2/Zenoh 节点 | 发布优先级 |
| --- | --- | --- | --- | --- |
| `split-mcu` | 量产机器人默认 | STM32/HPM/裸机或 RTOS | ARM Linux 计算机 | P0 |
| `split-linux-rt` | 高性能 ARM SoC | Linux PREEMPT_RT 用户态 ESOP | 同机或独立 Linux | P1 |
| `single-host-dev` | PC 开发、仿真、HIL | Linux raw port | 同进程或同机 | P0，仅开发用途 |

实时节点与 Linux 节点之间的 IPC 必须有版本化头、单调序号、时间戳、质量状态和掉线检测。实时节点不能等待 ROS 2 executor、Zenoh router、DNS、磁盘或远程网络。

## 4. 分层与软件包规划

### 4.1 分层

```text
Application control policy
  |-- robot state machine, per-axis limits, mode switch
  |-- no ROS, Zenoh, heap allocation or file IO in hard RT task

Robot device layer
  |-- esop_cia402, esop_io, esop_ft, esop_gripper, periph adapters

Data layer
  |-- esop_procbuf (RT state/command), esop_ipc, esop_proto

EtherCAT layer
  |-- esop_ecat_core, esop_coe, esop_dc, domains, diagnostics

Platform layer
  |-- port_stm32_eth, port_hpm_gmac, port_linux_raw, time/cache/IRQ
```

### 4.2 代码模块

| 模块 | 语言/运行位置 | 责任 | 直接依赖 |
| --- | --- | --- | --- |
| `esop_core` | Rust `no_std`，实时节点 | 生命周期、错误码、固定内存、事件环 | 无平台 SDK |
| `esop_ecat` | Rust `no_std`，实时节点 | 数据报、扫描、AL、Domain、固定容量 Domain/PDO 注册、WKC、PDO 调度 | `esop_core`、`esop_port` |
| `esop_coe` | Rust `no_std`，实时节点 | 邮箱、SDO、PDO 配置 | `esop_ecat` |
| `esop_dc` | Rust `no_std`，实时节点 | DC 拓扑、参考钟、SYNC0 与偏差诊断 | `esop_ecat`、单调时间 |
| `esop_profile_cia402` | Rust `no_std`，实时节点 | 驱动状态机、对象字典模板、PDO 映射、模式转换 | `esop_coe`、`esop_procbuf` |
| `esop_device` | Rust `no_std`，实时节点 | 驱动/传感器/IO 的统一能力与生命周期 | `esop_procbuf` |
| `esop_periph_*` | Rust `no_std`，实时节点 | I2C/SPI/UART/CAN-FD/GPIO/USB 外设适配 | `esop_device`、BSP port |
| `esop_procbuf` | Rust `no_std`，双域 | 固定布局 RT 缓冲、双页快照、事件 ring、质量位 | 独立 ABI 层；不引入平台依赖 |
| `esop_ipc` | C/C++，双域 | shared memory、RPMsg 或 Unix domain socket 的封装 | `esop_procbuf` |
| `esop_proto` | `.proto` + 生成代码，非实时域 | API、配置、状态、事件、记录数据定义 | protobuf runtime |
| `esop_zenoh_gateway` | C++/Rust/C，Linux | Protobuf pub/sub/query、远程状态与命令网关 | Zenoh、`esop_ipc` |
| `esop_ros2_control` | C++，Linux | `hardware_interface::SystemInterface` 插件，`read()`/`write()` 映射 ProcBuf | ROS 2、`esop_ipc` |
| `esop_ros2_bridge` | C++，Linux | ROS topic/service/action 与 ESOP Proto/诊断的显式映射 | ROS 2、`esop_ipc` |
| `esop_cfggen` | Rust/Python/C++，宿主机 | ESI/设备配置/URDF 约束输入生成静态 C 配置和 ProcBuf layout | 非固件工具依赖 |
| `esop_sim` | C++/Python，CI | 虚拟从站、PCAP 回放、ProcBuf 与 API 合约测试 | host 工具链 |

`esop_ecat`、`esop_coe`、`esop_dc`、`esop_profile_cia402`、`esop_device` 与 `esop_procbuf` 是 P0 实时闭环。Zenoh、ROS 2 和 protobuf runtime 绝不能进入这些模块的链接依赖图。

当前 `crates/esop-device/` 已提供固定容量的统一设备生命周期注册表和显式故障恢复迁移；profile、端口和外设驱动仍需在其上实现具体 probe/configure/cyclic 操作。

当前 `crates/esop-ethercat-core/src/domain_registry.rs` 已提供固定容量的多 Domain/PDO/datagram 注册层：它在激活前分配稳定 bit offset、校验过程映像和逻辑地址范围、生成多速率调度表，并在激活后锁定配置。`SiiConfigurationCandidate` 可冻结为 `SiiDomainProjection`，将方向局部 PDO 布局与已核验的 FMMU/SyncManager 映射事务式登记到统一 Domain；字节对齐的 segment 可自动绑定 `LWR`/`LRD`，`FramePlanSet` 可按 MTU 拆分并在激活时原子发布。它不替代真实 SII/ESI 自动发现或 FMMU/SM 硬件回读。

## 5. ProcBuf：机器人实时数据载体

### 5.1 目标

ProcBuf 是 ESOP 的稳定实时 ABI，用于连接：EtherCAT PDO Domain、设备 profile、实时控制器和 Linux/ROS 2 网关。它是**固定大小、预分配、生成式布局**的共享数据结构，而不是通用消息总线。

每个机器人配置由 `esop_cfggen` 生成：

- `esop_procbuf_layout.h`：C 结构、offset、size、对齐与 schema hash；
- `robot_esop.proto`：控制面/观测面的 Protobuf 消息；
- `robot_esop.yaml`：人可读布局与设备能力报告；
- EtherCAT 静态从站、PDO、DC、Domain 配置。

### 5.2 内存布局

```text
ProcBuf region
  Header: magic, ABI version, layout hash, robot ID, boot ID
  Command page (writer: controller/supervisor, reader: RT task)
    seq, desired_mode, motion_enable_request, joint command[N], gpio command[M]
  State page (writer: RT task, reader: supervisor)
    seq, ecat_time_ns, app_time_ns, joint state[N], IO state[M], health
  Quality page
    per-domain WKC, freshness, link, AL state, DC offset, fault bitmap
  Event ring
    fixed records: timestamp, source, severity, code, axis/device, args
```

`joint command` 和 `joint state` 使用明确的单位：SI 单位（rad、rad/s、Nm、A、V、degC）或由 device profile 明确记录的原始单位；同一字段不得在不同驱动中改变单位。所有 float/整数宽度、endianness 和 padding 都由生成器固定。跨 CPU 共享时使用明确的原子语义和 cache/内存屏障，而不能依赖 C struct 的偶然布局。

### 5.3 周期语义

1. Linux/上层控制把**下一周期**的命令写入 Command page，完成后以 release 语义更新 `command_seq`。
2. 实时任务在周期起点读取一个一致命令快照，验证 boot ID、序号、时效、模式、限幅和使能条件。
3. 合格命令被写入 EtherCAT RxPDO；不合格、过期或监督节点掉线时执行配置的 hold/ramp-to-zero/disable 策略。
4. RX PDO 与 WKC 合格后，实时任务更新 State 和 Quality page，再以 release 语义发布 `state_seq`。
5. 监督节点以 acquire 语义读取最新完整状态；它不能反向阻塞周期任务。

首版采用单写者/单读者的 sequence-lock 双页模型。多控制器仲裁在实时应用层完成，禁止多个 ROS 2 控制器同时直接写同一轴的 ProcBuf command 字段。

### 5.4 ProcBuf 与 Protobuf 的映射

| 数据类别 | 真实来源 | ProcBuf 表示 | Protobuf 表示 |
| --- | --- | --- | --- |
| 周期关节命令 | ROS 控制器或本地控制器 | 固定数组、序号和 deadline | `RobotCommand` 快照，仅用于网关/记录/调试 |
| 周期关节状态 | TxPDO | 固定数组、时间戳、quality | `RobotState` 快照或降采样流 |
| 驱动故障 | CoE/AL/状态字 | fault bitmap + event ring | `DeviceEvent` |
| 配置 | 设备清单、限制、PDO layout | 仅 layout hash/激活配置 ID | `RobotConfig`/`DeviceConfig` |
| 维护操作 | SDO、reconfigure、FoE | 请求 ID 和状态 | `MaintenanceRequest`/`MaintenanceResult` |

禁止将可变长 `bytes process_image` 当作机器人运行的正式 API。原始 PDO dump 只可用于抓包、诊断或回放；应用应看到带单位、轴名、时间戳与质量状态的语义数据。

## 6. CoE 与其他外设接入模型

### 6.1 Device/profile 插件契约

每个 EtherCAT 或非 EtherCAT 设备适配器实现同一生命周期：

```text
probe -> identify -> configure -> verify -> activate -> cyclic_read/write
      -> degraded/fault -> recover -> deactivate
```

| 插件类别 | P0 示例 | 运行时职责 |
| --- | --- | --- |
| `ecat_profile_cia402` | 关节伺服、轮毂电机、升降轴 | CoE 参数化、PDO 模板、CiA 402 状态机、CSP/CSV/CST、故障恢复 |
| `ecat_io` | DI/DO、AI/AO、编码器 | PDO 映射、量程/去抖、IO quality |
| `ecat_sensor` | 力/力矩、绝对编码器 | 时间戳、标定、量纲、输入质量 |
| `periph_canfd` | CAN-FD IMU、智能电池、末端工具 | 总线调度、命令/状态映射；不能假装有 EtherCAT DC 精度 |
| `periph_i2c_spi` | 板载 IMU、温度、电源监测 | 非阻塞驱动、采样缓存和故障状态 |
| `periph_uart_usb` | GNSS、调试工具、用户设备 | 帧协议和隔离队列；禁止直接阻塞 RT 周期 |
| `periph_gpio` | 限位、触发、使能、指示 | 去抖、边沿事件、输出 fail-safe 默认值 |

设备插件在配置/维护平面可进行异步 CoE SDO 或外设事务；`cyclic_read/write` 只处理预先绑定的固定字段，不允许堆分配、字符串解析、文件访问或等待总线事务完成。

### 6.2 CiA 402 关节接口

对每个 `joint` 生成最少以下语义字段：

| Command | State | Health |
| --- | --- | --- |
| requested mode | actual mode | CiA 402 state |
| target position/velocity/torque | position/velocity/torque actual | statusword/controlword |
| motion enable request | following error | drive error code |
| command deadline/sequence | timestamp/age | PDO WKC and DC quality |

驱动厂商对象（齿比、电子齿轮、转矩限制、滤波、回零方式）保留在 profile 的 typed 参数表中，由配置期 SDO 设置并读取回校验。不能为了方便把未经校验的任意 SDO 写入暴露给运动 ROS topic。

### 6.3 功能安全边界

急停、安全扭矩关断、碰撞限力和安全区域属于系统级安全功能。ESOP 可传递安全相关状态、在普通通信故障时停止非安全运动输出、并为安全 PLC/FSoE 集成预留接口；但普通 CoE/PDO/ROS 指令不构成认证安全通道。FSoE、认证驱动配置、安全 PLC 和安全验证是单独产品工作流。

## 7. Protobuf、Zenoh 与 ROS 2 集成

### 7.1 Protobuf 契约

在 `proto/esop/v1/` 保存唯一的数据契约源，先定义：

```text
robot.proto       RobotCommand, RobotState, JointState, Quality
device.proto      DeviceInfo, DeviceConfig, DeviceEvent, Fault
maintenance.proto SdoRequest, SdoResult, LifecycleRequest, LifecycleResult
diagnostic.proto  TraceEvent, PerformanceReport, BusTopology
```

规则：包名和 major version 进入路径；字段号永不复用；删除字段必须 `reserved`；枚举保留 `UNSPECIFIED` 和未知值处理；每条消息携带 `robot_id`、`boot_id`、`schema_version`、`monotonic_time_ns` 和 source sequence（当适用时）。Protobuf 支持字段演进，但不保证同一逻辑消息序列化为唯一字节串，因此不能把序列化字节比较或签名作为语义相等性判断。[S1]

发布物包含 `.proto`、C/C++/Python/Rust 生成绑定和 descriptor set。实时 MCU 可选 nanopb 等受控实现，但只用于非周期控制/诊断路径，且其版本和最大编码长度必须在构建时锁定。

### 7.2 Zenoh 网关

`esop_zenoh_gateway` 运行于 Linux 监督域，负责发布 Protobuf、接收授权命令、提供查询，并把它们转换为 `esop_ipc` 请求。推荐 key expression：

```text
esop/<fleet>/<robot_id>/state
esop/<fleet>/<robot_id>/event/**
esop/<fleet>/<robot_id>/diagnostic/**
esop/<fleet>/<robot_id>/cmd
esop/<fleet>/<robot_id>/query/**
```

| Zenoh 数据 | 方向 | 可靠性/频率原则 | RT 影响 |
| --- | --- | --- | --- |
| `state` | ESOP -> 上层 | 最新状态；可降采样为 50-250 Hz | 无；读取 State page 快照 |
| `event` | ESOP -> 上层 | 可靠、带事件 ID；允许补拉 | 无；从 event ring 异步取数 |
| `diagnostic` | ESOP -> 上层 | 低优先级、限流 | 无 |
| `cmd` | 上层 -> ESOP | 有 TTL、来源认证、序号与 ACL | 网关写 Command page；RT 验证 deadline |
| `query` | 双向 | 配置、拓扑、历史、维护结果 | 仅控制/维护平面 |

Zenoh key expression 是路由和匹配的基础抽象；网关将 Protobuf 作为 payload，而不是由 key 名称推断消息结构。[S4]首版使用 Linux `zenoh-c`/`zenoh-cpp` 或受支持绑定。现有 `zenoh-c` 的构建依赖 Rust 工具链，因此不能把它视作 MCU 核心的零依赖组件。[S5]

### 7.3 ROS 2 集成策略

ROS 2 的接入分两类：

1. **运动控制路径：**`esop_ros2_control` 实现 `hardware_interface::SystemInterface`。`read()` 从 ProcBuf State page 更新 `state_interfaces`；`write()` 把 controller 输出写入 Command page。`controller_manager` 的实时 read-update-write 循环与该模型相匹配。[S6]
2. **系统集成路径：**`esop_ros2_bridge` 把机器人状态、诊断和维护操作映射为 ROS 2 topics、services、actions；它不直接碰 EtherCAT 端口，也不拥有 ProcBuf 的实时写权。

ROS 2 硬件组件本身区分 actuator、sensor 和 system，也支持不同读写频率；ESOP 应以一个机器人 `SystemInterface` 或按子系统分组的 hardware component 暴露，而不是让每个 EtherCAT 从站成为独立 ROS executor。[S7][S8]

ROS 2 进程可选用 DDS RMW 或 `rmw_zenoh_cpp`。若使用后者，部署必须包含可用 Zenoh router：该 RMW 默认依赖 router 完成发现，并以 CDR 保持 ROS 2 类型兼容，不会把 ROS 2 topic 自动改为 Protobuf。[S3][S9]因此：

- ROS 2 topic/action/service 保持 `rosidl` 类型和 CDR；
- ESOP 原生外部 API 使用 versioned Protobuf；
- 两者通过明确的 `esop_ros2_bridge` 进行字段映射，禁止隐式双向泛型转换；
- 运行时只能为一个 ROS 2 进程选择一个 RMW；DDS 与 Zenoh 的互操作需求应通过独立 bridge/router 设计验证。

## 8. 时间、QoS 与故障模型

### 8.1 时钟域

| 时钟 | 所在域 | 用途 | 不可替代为 |
| --- | --- | --- | --- |
| EtherCAT DC time | 现场总线 | PDO/从站同步、SYNC0、驱动采样关联 | ROS time |
| ESOP monotonic time | 实时节点 | cycle deadline、命令时效、事件排序 | wall clock |
| Linux monotonic time | 监督节点 | IPC 延迟、记录、网关 TTL | EtherCAT DC 的精确替代 |
| ROS time | ROS 2 图 | 仿真/可视化/消息时间 | 实时周期基准 |

ProcBuf State 同时携带 `ecat_time_ns`、`esop_monotonic_time_ns` 和转换质量/偏差。只有经过校准的映射才可将外部 ROS 时间用于分析；绝不以 ROS 时间调度 PDO。

### 8.2 命令与失效策略

| 事件 | 实时节点默认动作 | 上层可见状态 |
| --- | --- | --- |
| Command page 超时 | 保持或按每轴策略斜坡到零，再请求 disable | `COMMAND_STALE` |
| WKC 连续异常 | 当前输入标记无效；冻结/撤销对应输出按策略 | `DOMAIN_DEGRADED` |
| 驱动离开 OP/产生 fault | 取消运动使能，触发 profile recovery FSM | `DRIVE_FAULT` |
| DC 偏差超阈值 | 记录、降级或拒绝高精度模式 | `CLOCK_DEGRADED` |
| Zenoh/ROS 2 断线 | 不影响已在 ProcBuf 中且未超时的周期；超时后按命令策略处理 | `SUPERVISOR_OFFLINE` |
| IPC boot ID 改变 | 丢弃旧命令，要求新的 lifecycle handshake | `SUPERVISOR_RESTARTED` |

运动命令必须有 source、sequence、deadline 和授权角色。网关 ACL、ROS 2 权限和设备状态机共同决定是否接受，不允许任意网络发布者直接改变 `motion_enable_request`。

## 9. 交付路线

| 里程碑 | 交付内容 | 验收门槛 |
| --- | --- | --- |
| R0：契约与仿真基线 | 目录结构、ProcBuf ABI、`.proto` v1、设备模型、PCAP/虚拟驱动仿真 | 同一 layout 从生成器产生 C header/YAML/proto descriptor；ABI/Schema 兼容检查在 CI 通过。 |
| R1：机器人 EtherCAT 实时节点 | `esop_ecat`、CoE、DC、ProcBuf、CiA 402 单轴和分布式 IO，STM32/HPM/Linux test ports | 1/8 轴驱动 + IO 达到 OP；500 us/1 ms 目标周期的 WKC、jitter、无分配报告通过。 |
| R2：多设备与鲁棒性 | 多 Domain、多速率、外设插件框架、事件环、诊断、恢复策略、配置生成 | EtherCAT + CAN-FD/I2C/SPI 的设备可同一 ProcBuf 表达；故障注入不破坏 RT 周期。 |
| R3：Zenoh/Protobuf 网关 | `esop_proto`、gateway、ACL、query、记录回放、fleet key namespace | 网络丢失、重连、schema 升级、命令 TTL 和授权拒绝测试通过。 |
| R4：ROS 2 控制接入 | `esop_ros2_control`、ROS bridge、URDF/ros2_control 配置生成、DDS 与 Zenoh RMW 测试矩阵 | `joint_trajectory_controller` 驱动仿真和实机；read/write 不分配、不等待网络。 |
| R5：产品扩展 | 力控接口、FoE、EoE/SoE/VoE、冗余、FSoE 项目集成 | 每个扩展独立编译开关，提供对周期、RAM、Flash 和故障模型的影响报告。 |

### 9.1 首个垂直切片

第一个可演示系统应避免同时做全功能机器人：

```text
Linux ARM host + STM32/HPM real-time node
  -> 2 个 CoE/CiA402 EtherCAT 伺服
  -> 1 个 EtherCAT DI/DO 模块
  -> 1 个 ProcBuf layout
  -> Zenoh state/event/cmd gateway (Protobuf)
  -> ROS 2 ros2_control SystemInterface
  -> 一个 joint trajectory 示例和断线/驱动 fault 演示
```

它一次验证全部关键边界：CoE/PDO/DC、实时数据与 protobuf 分离、Zenoh、ROS 2、IPC、运动使能和故障行为。

## 10. 可验证需求

| ID | 优先级 | 需求 | 验收证据 |
| --- | --- | --- | --- |
| ROB-001 | P0 | 实时 EtherCAT 周期路径不得链接 protobuf、Zenoh、ROS 2、文件系统或动态分配器。 | 链接依赖清单与 30 分钟分配拦截测试。 |
| ROB-002 | P0 | CoE SDO、静态 PDO 配置、CiA 402 状态机、CSP/CSV/CST 选择和 DC 必须可由 profile 配置。 | 两种不同厂商驱动的启动/状态/故障 HIL 测试。 |
| ROB-003 | P0 | 每个关节的命令、状态、时间戳、quality、enable 与 fault 必须映射到 ProcBuf。 | 生成 layout 与实物 PDO offset 比对。 |
| ROB-004 | P0 | ProcBuf 读写满足单写者/单读者一致性；读者不得看到部分 state，过期命令不得进入 PDO。 | race、重启、cache 和 deadline 注入测试。 |
| ROB-005 | P0 | Device/profile 插件必须能描述 EtherCAT 和非 EtherCAT 外设，且非周期外设不能阻塞 PDO 周期。 | CAN-FD 或 I2C 故障压力下周期 P99 jitter 报告。 |
| ROB-006 | P0 | 驱动/WKC/AL/DC/命令失效必须形成结构化事件并改变 ProcBuf quality。 | 五类故障注入 HIL。 |
| ROB-007 | P1 | Protobuf API 必须使用 versioned package，字段演进兼容检查自动执行。 | old writer/new reader 与 new writer/old reader CI 组合。 |
| ROB-008 | P1 | Zenoh gateway 必须执行 key namespace、命令 TTL、source identity、ACL 和限流。 | 未授权、重放、过期、断连重连测试。 |
| ROB-009 | P1 | `esop_ros2_control` 的 `read()`/`write()` 只访问 ProcBuf/IPC，不直接访问 EtherCAT 或网络。 | 单元测试、依赖检查和 controller HIL。 |
| ROB-010 | P1 | ROS 2 使用 Zenoh RMW 时，router 存活、发现失败和版本固定必须是部署健康检查项。 | router 不可达/恢复的系统测试。 |
| ROB-011 | P1 | 每个机器人 build 输出静态内存、ProcBuf 大小、PDO 带宽、预期 WKC、周期预算和设备清单。 | CI 生成并审核 `robot_build_report.json`。 |
| ROB-012 | P2 | FSoE/安全 PLC 集成须形成独立安全需求、测试和证据包，不复用普通 EtherCAT 验收结论。 | 安全项目独立评审通过。 |

## 11. 测试、可观测性和发布标准

### 11.1 测试层次

1. `wire/unit`：数据报、CoE、CiA 402、ProcBuf 原子语义、Protobuf schema 兼容性。
2. `simulation`：虚拟 EtherCAT 从站、驱动状态和 PCAP 回放；ROS 2 controller 与 Zenoh gateway 的契约测试。
3. `HIL-basic`：单轴/双轴 + IO，扫描、配置、OP、PDO、SDO、DC。
4. `HIL-fault`：拔线、断电、WKC 异常、驱动 fault、DC 偏差、IPC/Zenoh/ROS 2 重启。
5. `soak/performance`：目标周期持续 30 分钟；记录 max/P99 jitter、WKC、RX/TX overflow、DC offset、堆分配、CPU 与温度。

### 11.2 关键遥测

`esop_diag` 至少导出：每 Domain 的 expected/actual WKC、last valid state age、cycle duration、deadline miss、DC offset、link/AL 状态、CiA 402 state、fault code、command age、ProcBuf sequence gap 和 IPC/gateway dropped count。

上述原始指标先进入 event ring/ProcBuf，再由非实时网关发布。禁止在实时任务中格式化 JSON、Protobuf 或文本日志。

### 11.3 发布阻塞条件

- R1 前：无动态分配周期路径、CoE/PDO/DC/ProcBuf HIL、驱动故障行为均有证据。
- R3 前：所有外部命令有身份、权限、TTL、序号和审计事件；Zenoh 断连不会让实时周期卡住。
- R4 前：ROS 2 `read/update/write` 端到端演示不绕过 ProcBuf；ROS 2 网络 QoS 不被当作安全或 EtherCAT 实时保证。
- 产品前：声明支持的驱动、模式、外设、周期、板卡、RTOS、ROS 2 distro、Zenoh/RMW 版本均纳入兼容矩阵。

## 12. 默认决策与待确认项

| 项目 | 默认决策 | 需要产品确认 |
| --- | --- | --- |
| `proctbuf` 含义 | 按 `protobuf` 解释；实时载体另命名 `ProcBuf` | 是否存在既有 `proctbuf` 协议/仓库 |
| 量产拓扑 | MCU/HPM/STM32 实时节点 + ARM Linux 监督节点 | 是否必须单 SoC 或纯 MCU 运行 |
| 首个机器人 | 2-8 轴 CiA 402 关节 + EtherCAT IO | 实际驱动品牌、PDO 与控制模式 |
| 周期目标 | 1 ms 基线，500 us 作为验证目标 | 控制带宽和轴数决定最终指标 |
| ROS 2 接入 | `ros2_control` SystemInterface + bridge | 目标 ROS 2 distro 与是否必须 `rmw_zenoh_cpp` |
| Zenoh 位置 | Linux gateway，MCU 不直连 | 是否需要远程车队/跨 WAN 路由 |
| 安全 | 普通控制与认证安全分域 | FSoE、安全 PLC、STO 和法规范围 |

## 13. 参考依据

- [S1] Protocol Buffers Documentation, `Overview` and schema evolution guidance, protobuf.dev, accessed 2026-09-03.
- [S2] ROS 2 Documentation, `rmw` middleware abstraction and RMW implementation guidance, docs.ros.org, accessed 2026-09-03.
- [S3] ROS 2 `rmw_zenoh` design documentation, ROS 2 / GitHub, accessed 2026-09-03.
- [S4] Eclipse Zenoh Documentation, `Abstractions` and key expressions, zenoh.io, accessed 2026-09-03.
- [S5] Eclipse Zenoh `zenoh-c` repository and build documentation, accessed 2026-09-03.
- [S6] ROS 2 Control documentation, hardware `read()`/`write()` real-time loop and `SystemInterface`, control.ros.org / ros-controls, accessed 2026-09-03.
- [S7] ROS 2 Control documentation, hardware component types, accessed 2026-09-03.
- [S8] ROS 2 Control documentation, different update rates for hardware components, accessed 2026-09-03.
- [S9] ROS 2 Documentation, Zenoh as an RMW vendor and router/discovery behavior, docs.ros.org, accessed 2026-09-03.
