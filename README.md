# ESOP

**ESOP = EtherCAT Simple Operating System**

ESOP 是面向嵌入式实时控制的 EtherCAT 简易操作系统；EtherCAT 主站是其首个核心组件。

项目的第一份交付物是基于 SOEM 与 IgH EtherCAT Master 源码分析形成的主站需求文档：

- [EtherCAT 主站需求与架构说明](docs/ethercat-master-requirements.md)
- [机器人 ESOP 软件规划](docs/robotics-esop-software-plan.md)
- [ESOP 实时与性能架构决策](docs/esop-performance-architecture-decision.md)
- [ETG.1500、Beckhoff 与 CiA 402 主站决策](docs/esop-etg-cia402-master-requirements.md)
- [eBPF 运行时观测与问题归因设计](docs/esop-ebpf-runtime-observability.md)

Linux 观测适配器位于 `crates/esop-ebpf-runtime/`：它使用 Rust/Aya 加载预编译 CO-RE BPF ELF，按能力挂载 tracepoint，从 ringbuf 解码固定证据并送入 `RuntimeAgent`。内核程序源和构建入口位于 `bpf/`；实时主站核心不依赖 Aya，也不等待观测器。

在具备 clang、bpftool 和 `/sys/kernel/btf/vmlinux` 的 Linux 主机上，可执行 `make -C bpf` 生成 BPF ELF，再用 `cargo run -p esop-ebpf-runtime --example observe -- bpf/build/esop_runtime.bpf.o` 启动只读观测进程。生产集成应由 ESOP 监督器提供真实 `boot_id`、`agent_epoch` 和每周期 `CycleContext`。

本地质量门统一由 `make ci` 执行，包含 workspace 测试、Clippy、release 构建、`aarch64-unknown-none` 核心检查和 BPF C 语法检查；`make setup-rust` 安装交叉检查目标，`make test-hil` 只运行 Linux 模拟端口 HIL 测试，`make bpf` 执行需要 clang 和内核 BTF 的完整 CO-RE 编译。

主站需求文档定义了面向 STM32、HPMicro 和通用 ARM/RISC-V 平台的最小依赖 EtherCAT 主站。当前第一阶段已经开始实现 Rust `no_std` 协议核心，位于 `crates/esop-ethercat-core/`，覆盖调用方固定 arena、帧/数据报编解码、固定帧池、O(1) RX index、有界收发周期、静态 PDO Frame Plan 到 Domain 的接收提交路径、多速率预计算调度、控制请求闭环、在线扫描、ESC 地址和基础信息读取、AL 单步状态转换、SII 身份读取、固定容量 EEPROM 分块读取、SII SyncManager/RxPDO/TxPDO category 只读解析与事务式固定容量配置候选投影、按 PDO 类别分段的多 SyncManager FMMU 逻辑地址分配、启动编排、静态 SM/FMMU 映射校验与写入读回、固定容量 CoE PDO assignment/mapping 写序列、PDO 位域访问、固定二进制诊断事件环、SPSC 无锁环、固定容量 DMA TX/RX 描述符所有权环与缓存维护契约、Frame Plan 到 DMA TX descriptor 的零中间帧拷贝构建、DMA TX 提交端口契约、DMA RX descriptor 直接消费会话、Mailbox 轮询与有限预算重试、协议错帧恢复、可配置 Status Bit 轮询、CoE SDO expedited/segmented 事务、异步 CoE Emergency 固定事件环、DC SYNC0/SYNC1 配置 FSM、FRMW reference-clock 周期同步槽、offset/jitter 锁定监测，以及多 Domain 的固定容量 PDO/datagram 注册、SII segment datagram 绑定、按 MTU 拆帧与原子激活编排。`crates/esop-profile-cia402/` 已提供独立的 CiA 402 Statusword FSA 解码、Controlword 使能序列、生命周期拒绝、Fault reset 单脉冲基础实现，以及可选 `ethercat` feature 下的标准 CiA 402 周期 PDO 对象绑定和 CSP/CSV/CST typed raw codec。`crates/esop-ethercat-linux-port/` 提供 Linux AF_PACKET 开发/HIL 端口、固定容量确定性 `SimulatedPort` 和显式链路状态刷新，`crates/esop-lifecycle-guard/` 提供独立的 fail-closed motion permit 守卫。完整 SII/ESI 自动发现、真实从站 PDO 互操作、DC 拓扑传播延迟与全从站运行时同步、厂商 quirk、STM32/HPMicro 平台 DMA 端口和真实设备 HIL 仍属于后续实现阶段。

`crates/esop-procbuf/` 提供固定布局的 ProcBuf ABI：Header/layout hash、Command/State 双页快照、Quality/Lifecycle/Runtime observation 数据和固定容量事件环。它只解决实时域内的固定数据交接，不等同于 shared-memory/RPMsg/UDS IPC，也不引入 Protobuf、Zenoh 或 ROS 2。

`crates/esop-profile-cia402/` 现已增加 CSP/CSV/CST 模式切换监督、实际模式确认、Operation Enabled 门槛、周期设定值首目标/限幅守卫，以及 `--features ethercat` 下复用核心 `PdoEntry` 的标准对象绑定。`write_control` 与 `write_cyclic` 分离；后者同时要求生命周期许可、模式确认、Operation Enabled 和 setpoint 有效。厂商 quirk 和真实驱动互操作仍需 HIL 验证。

`crates/esop-device/` 提供 EtherCAT 驱动、IO 和非 EtherCAT 外设共享的固定容量生命周期注册表，强制执行 probe、identify、configure、verify、activate、cyclic、degraded/fault、recover 和 deactivate 的显式迁移。

`DomainRegistry` 位于 `esop-ethercat-core` 内，负责在激活前登记多个 Domain、PDO entry 和 datagram，校验过程映像/逻辑地址不重叠，生成多速率 `ScheduleTable`，并在激活后锁定配置。PDO offset 在 Domain 内稳定，datagram 计划使用全局过程映像偏移；输入段可直接转换为现有 `Domain` 的 staging 描述。SII 配置候选可先冻结为 `SiiDomainProjection`，校验 Rx/Tx 分区、FMMU/SyncManager 物理映射和逻辑基址后，再由注册表一次性事务式登记；字节对齐的 SII segment 可自动生成 `LWR`/`LRD` datagram。`FramePlanSet` 在激活期按 datagram 容量和 MTU 固定拆帧，并与注册表一起原子发布，失败时不锁定配置。

当前工作区另有 `crates/esop-lifecycle-guard/`，提供独立的固定门槛和 motion permit 生命周期守卫；`crates/esop-ebpf-agent/` 提供固定证据 ABI、问题相关器、健康心跳和 eBPF 能力预检结果模型。它们只负责普通控制域的 fail-closed 策略和运行时观测，不替代 STO、FSoE、安全 PLC 或认证安全通道。
