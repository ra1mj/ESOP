# ESOP EtherCAT 主站与 CiA 402 完整决策

- 文档版本：1.0
- 日期：2026-09-03
- 状态：已接受的产品能力决策，待代码和一致性证据实现
- 产品全名：`ESOP = EtherCAT Simple Operating System`
- 相关文档：[主站基础需求](ethercat-master-requirements.md)、[性能 ADR](esop-performance-architecture-decision.md)、[机器人软件规划](robotics-esop-software-plan.md)
- 规范目标：**ETG.1500 Class B v1.0.2 基本功能集 + FP Motion Control v1.0.0**
- Drive profile 基线：ETG.6010 v1.1.0；CiA 402-1/-2/-3 v5.0.0（2024），最终实现以项目合法取得的规范正文为准

## 1. 结论

一个可用于机器人伺服的 EtherCAT 主站，不只是“能发 EtherCAT 帧”的网卡程序。ESOP 至少需要以下完整链路：

1. 合格的 100 Mbit/s Ethernet MAC/PHY、DMA、定时器、cache 和中断端口。
2. EtherCAT 帧/数据报引擎、寻址、帧聚合、匹配、超时和 Working Counter 校验。
3. 网络扫描、SII/EEPROM、ESI/静态配置、实际拓扑与预期配置比较。
4. INIT、PREOP、SAFEOP、OP、错误确认和特殊行为完整状态机；固件下载 capability 还需 BOOT。
5. SyncManager、FMMU、watchdog、PDO、过程映像、Domain 和确定性周期调度。
6. Mailbox、Mailbox Resilient Layer、输入邮箱轮询和异步事务调度。
7. CoE SDO、segmented transfer、Emergency 和 PDO 配置。
8. DC 初始化、传播延迟、offset、start time、drift compensation 和同步质量监测。
9. CiA 402 驱动状态机、CSP/CSV/CST、对象绑定、模式切换和故障策略。
10. 配置生成器、诊断、恢复、HIL、性能资格和一致性声明管理。

**当前状态：仓库已包含 Rust `no_std` EtherCAT 核心、Linux AF_PACKET 开发/HIL 端口、固定 SPSC ring、固定诊断事件环、控制请求闭环、单 Domain PDO 接收提交路径、扫描/SII/AL 基础状态机、SII SyncManager/RxPDO/TxPDO category 只读解析及事务式固定容量配置候选、SM/FMMU 写入读回配置 FSM、固定容量 CoE PDO assignment/mapping 写序列、独立生命周期守卫、Mailbox 轮询 FSM、有限预算重试与协议错帧恢复、可配置 Status Bit 轮询、CoE SDO expedited/segmented codec/事务 FSM、异步 CoE Emergency 固定事件环，以及 DC SYNC0/SYNC1 配置 FSM、FRMW reference-clock 周期同步槽和 offset/jitter 监测器。固定容量 `DomainRegistry` 已支持多 Domain/PDO/datagram、SII 字节对齐 segment 的 `LWR`/`LRD` 绑定、按 MTU 拆帧和原子激活。另有独立 `esop-profile-cia402` crate，已实现 Statusword FSA 解码、基础 Controlword 使能序列、生命周期拒绝和 Fault reset 单脉冲。仍未形成完整主站、完整 SII/ESI 自动发现、真实从站 PDO 互操作、DC 拓扑传播延迟/全从站运行时同步、CiA 402 模式/CSP-CSV-CST、MCU DMA 端口。**现有内容是架构与验收基线，不是 ETG 认证证据。

ProcBuf 的固定 ABI、双页 Command/State 快照、Quality/Lifecycle/Runtime observation 和事件环已在 `esop-procbuf` crate 落地；它尚未替代 shared-memory/RPMsg/UDS IPC 或真实 MCU 端口。

CiA 402 profile 已增加 CSP/CSV/CST 的模式切换监督、实际模式确认、Operation Enabled 门槛和周期设定值首目标/限幅守卫；完整 PDO 对象绑定、厂商 quirk 和真实驱动互操作仍需 HIL 证据，不能据此宣称三种模式已完成互操作资格。

## 2. 规范边界

### 2.1 谁定义什么

| 来源 | 对 ESOP 的作用 | 不能混淆的边界 |
| --- | --- | --- |
| ETG.1000 系列 | EtherCAT Data Link/Application Layer、帧、数据报、ESM、Mailbox 等协议基础 | 是 EtherCAT 核心规范，不等于 CiA 402 驱动 profile |
| ETG.1020/2000/2100/8000 | 协议增强、ESI、ENI、知识库和 DC/错误处理细节 | 具体版本和可访问范围需在项目合规清单中锁定 |
| ETG.1500 v1.0.2 | 定义 Class A、Class B 和 Feature Pack 的主站能力集合 | 它是能力分类，不替代 ETG.1000 的逐字段实现要求 |
| ETG.6010 | 规定 CiA 402 drive profile 在 EtherCAT 上的共同实现行为 | 它连接 EtherCAT 与 CiA 402，不是通用从站的强制 profile |
| CiA 402 系列 / IEC 61800-7 | 定义驱动状态机、Controlword/Statusword、模式和对象 | 属于设备/应用 profile，可位于主站应用或 ESOP profile 层 |
| Beckhoff InfoSys | 解释 ESM、CoE、DC、WKC、SII 和实际设备行为，作为互操作参考 | Beckhoff 文档不是 ESOP 可以替代购买/授权规范的依据 |
| IgH EtherCAT Master | 参考 Domain、显式周期 API、异步 FSM 和性能机制 | 只能参考公开行为与思想，ESOP 必须独立实现 |

截至 2026-09-03，ETG 官方公开下载仍将 ETG.1500 标为 `D (R) 1.0.2`。CiA 官方页面列出的当前 CiA 402 系列为 2024 年发布的 `CiA 402-1/-2/-3 version 5.0.0`。产品配置必须记录实际采用的规范基线，不可把对象集和厂商行为永久固定在旧版本假设上。

### 2.2 Beckhoff 与 CiA 要求的正确理解

Beckhoff 是 EtherCAT 技术许可方，Beckhoff InfoSys 对 TwinCAT 和 Beckhoff 终端的说明可帮助验证主站行为，例如：

- ESM 必须按 INIT、PREOP、SAFEOP、OP 管理通信和应用状态；
- WKC 必须与命令和从站参与情况对应，异常时输入不能当作有效数据发布；
- CoE 通过 EtherCAT Mailbox 访问 CANopen 对象字典；
- DC 由主站测量拓扑传播延迟、选择参考钟并持续同步；
- SII/EEPROM 和 ESI 提供身份、邮箱、SM、FMMU、PDO、DC 和状态转换信息。

但“CiA 要求”应以 CiA 402、IEC 61800-7 和 ETG.6010 为准。不能因为某个 Beckhoff 驱动或 TwinCAT 示例接受一种对象映射，就把它当作所有 CiA 402 驱动的共同强制行为。

## 3. 产品等级决策

### DEC-MST-001：Class B + Motion Control FP

首个可发布的 ESOP 机器人主站按以下能力声明设计：

```text
ETG.1500 Basic Feature Set: Master Class B, version 1.0.2
ETG.1500 Feature Pack: Motion Control, version 1.0.0
Drive profile: CiA 402 over CoE
Synchronization: Distributed Clocks
```

选择 Class B 的原因是 ESOP 首发运行于资源受限 MCU。ETG.1500 本身建议实现尽量达到 Class A；因此 Class B 是发布下限，不是长期上限。Class A、Cable Redundancy、EoE、FoE、SoE 和 Master Object Dictionary 在后续 capability 中独立演进。

### DEC-MST-002：能力声明不等于认证

在完成适用的 ETG 一致性测试、互操作测试和文档审查之前，对外只能使用：

> Designed and internally tested against the ETG.1500 Class B v1.0.2 capability baseline and Motion Control Feature Pack v1.0.0 requirements.

不得使用“ETG certified”“officially conformant”或容易被理解为正式认证的表述。发布 manifest 必须列出规范版本、实现功能、条件功能、未实现功能、板卡、PHY、驱动和测试拓扑。

### DEC-MST-003：配置逻辑与实时主站分离

ETG.1500 区分 configuration logic 与 EtherCAT master runtime。ESOP 采用：

- `esop_cfggen` 在宿主机读取 ESI、产品配置和设备 profile，生成静态配置；
- 主站启动时仍在线扫描 SII/ESC 并比较实际网络；
- MCU 固件不运行大型 XML/ENI 解析器；
- 首版不声明正式 ENI import capability；若未来声明，必须补齐 ENI InitCmd validate 和 Complete Access 等条件要求。

### DEC-MST-004：CiA 402 是独立 profile

CiA 402 状态机和厂商兼容策略位于 `esop_profile_cia402`，不得进入 EtherCAT frame、AL、Mailbox、CoE 或 DC 核心。通用 EtherCAT IO、传感器、编码器和网关不应被伪装成 CiA 402 驱动。

## 4. 一个主站需要的完整能力

### 4.1 硬件与平台端口

EtherCAT 主站通常可使用标准 Ethernet 控制器，不要求主站侧 EtherCAT 专用 ASIC；但机器人实时主站仍必须满足确定性端口条件。

| 能力 | P0 要求 |
| --- | --- |
| Ethernet | 100 Mbit/s 全双工，原始二层 EtherType `0x88A4` 收发，禁用会改变帧时序/内容的 offload |
| MAC/PHY | 链路状态、MDIO、RMII/MII 时钟、错误计数可诊断；目标板和 PHY 组合经过资格测试 |
| DMA | 静态 TX/RX descriptor ring；调用方提供对齐缓冲；明确 CPU/DMA 所有权 |
| Cache/MPU | descriptor 和 payload 的 clean/invalidate、barrier、cache-line 对齐及 DMA 可达内存区 |
| 时间 | 单调高分辨率定时器、绝对周期释放；DC application time 不使用 wall clock |
| IRQ | ISR 只确认 DMA/错误并唤醒 RT task；协议解析、CoE 和回调不在 ISR 运行 |
| 原子能力 | 声明 8/16/32/64 位原子是否 lock-free；不可用时采用有界 sequence counter 或短临界区 |
| 资源 | arena、ProcBuf、frame、mailbox、event 和 stack 大小在构建时输出，激活后不分配 |

### 4.2 EtherCAT 数据链路引擎

主站需要：

- 构造/解析 Ethernet、EtherCAT 和 datagram header；
- 支持 APRD/APWR/APRW、FPRD/FPWR/FPRW、BRD/BWR/BRW、LRD/LWR/LRW、ARMW、FRMW；
- 在单帧聚合多个数据报并处理 follow、length、index、IRQ 和 WKC；
- 自动递增、固定物理、广播和逻辑寻址；
- 为每个在途 frame/datagram 维护 generation、deadline、期望地址/长度/命令和 expected WKC；
- 拒绝短帧、超长帧、旧帧、重复帧、未知 index、地址/长度/类型不匹配；
- 只在完整匹配并通过 WKC 策略后提交输入过程映像。

### 4.3 发现、SII 与配置

P0 启动流程：

```text
link up
  -> broadcast/auto-increment scan
  -> read ESC capabilities and DL status
  -> read SII identity/mailbox/SM/FMMU/PDO/DC data
  -> assign configured station addresses
  -> compare topology and identities with static configuration
  -> configure SM/FMMU/watchdog/PDO/mailbox/DC
  -> verify writes and transition ESM
```

比较项至少包含 vendor ID、product code、revision、serial 策略、alias/position、拓扑和显式 identification。配置不匹配时默认禁止进入 OP，不允许“尽量运行”掩盖换线或错设备。

### 4.4 EtherCAT State Machine

主站必须管理 INIT、PREOP、SAFEOP、OP；实现固件下载时还需 BOOT。状态机必须包括：

- 请求状态、读取实际状态、AL Error Indication Acknowledge 和 AL status code；
- 使用 ESI/SII 的状态转换 timeout，缺失时使用受版本控制的 ETG 默认值；
- Device Emulation 的特殊 error acknowledge 行为；
- `OpOnly` 设备在非 OP 时禁用所有 output SyncManager；
- 部分从站失败、全网降级、重试次数和回退状态的显式策略；
- 所有转换异步推进，不在 cycle API 中 sleep 或无限轮询。

### 4.5 PDO、Domain 与周期调度

| 能力 | 要求 |
| --- | --- |
| SM/FMMU | 按设备能力配置方向、物理地址、逻辑地址、enable 和 watchdog |
| PDO mapping | 支持静态 PDO assignment/mapping；配置期 SDO 写入后 read-back |
| Process image | 固定布局、调用方提供内存、bit offset 可验证；输入提交带质量状态 |
| Domain | 多 Domain、独立周期/相位、expected/actual WKC 和 input age |
| Scheduling | 激活时生成 hyperperiod、Frame Plan 和 wire budget；周期内不动态装箱 |
| Slave-to-Slave | 按配置由主站复制数据，携带源质量，最大路径不超过两个周期 |
| Watchdog | 输出、SM 和应用命令时效联合管理；故障动作由产品安全策略决定 |

### 4.6 Mailbox 与 CoE

Mailbox 不属于硬实时 PDO，但属于 Class B P0：

- mailbox header、counter、send/receive、timeout、error response；
- 与 CoE/FoE/SoE 等上层无关的 Mailbox Resilient Layer；
- 按 PollTime 读输入 mailbox，或通过 FMMU 映射 Mailbox Status Bit 后按需读取；
- 每个请求使用固定缓冲和异步 FSM，周期任务只给出 byte/datagram/time 预算；
- CoE expedited/normal SDO upload/download；
- segmented SDO 作为 ESOP P0，避免对象超过 mailbox 长度时失去互操作能力；
- Complete Access、SDO Information 作为 P1，但 profile 可声明设备需要；
- 接收 CoE Emergency 并写入固定事件环；消费不及时不能阻塞 mailbox；
- FoE/EoE/SoE 只有产品声明支持时，才启用对应 capability 和测试。

### 4.7 Distributed Clocks

Motion Control Feature Pack 要求 DC。完整 DC 不是只写 SYNC0 周期，还包括：

1. 识别 DC capable slave 和端口拓扑。
2. 初始 propagation delay measurement。
3. 各从站 system time offset compensation。
4. 选择 reference clock，设置 start time、SYNC0/SYNC1 周期与 shift。
5. 周期 drift compensation，并使 master/application time 跟随参考钟策略。
6. 读取 `0x092C` 等同步差异信息，监测 sync window。
7. 记录 offset、jitter、last sync、失锁次数和连续异常。
8. DC 未锁定时阻止新的有效 CSP/CSV/CST 运动目标，按策略 hold、ramp 或 disable。

### 4.8 诊断与恢复

P0 诊断至少提供：

- master/link/slave/Domain/drive 状态快照；
- 每帧/Domain expected WKC、actual WKC、timeout、unmatched、corrupt；
- AL state/status code、ESC error register、mailbox/SDO abort、Emergency；
- DC offset/jitter/lock、cycle duration、release jitter、deadline miss；
- CiA 402 state、Statusword、error code、mode、following error；
- command age、input age、event dropped count、RX overflow、TX starvation；
- rescan、reconfigure、request state 和 fault reset 的受预算异步 API。

恢复必须由应用策略明确允许。默认不在拓扑变化或 WKC 异常后自动恢复运动输出并返回 OP。

## 5. ETG.1500 Class B 对照

下表只给出产品决策，详细协议行为仍以适用规范为准。

| ETG.1500 功能 | Class B | ESOP 决策 | 优先级 |
| --- | --- | --- | --- |
| Service Commands | ENI import 时 shall | 实现主站所需全部常用命令；首版不声明 ENI import | P0 |
| IRQ field | should | 解析并保留；拓扑事件监测 P1 | P1 |
| Device Emulation | shall | 正确处理特殊 AL acknowledge | P0 |
| ESM special behaviour | shall | ESI/SII timeout、OpOnly、错误确认 | P0 |
| Error Handling/WKC | shall | frame/Domain/slave 分层 WKC 和质量提交 | P0 |
| EtherCAT frame type | shall | 原始 EtherType `0x88A4`、Type 1 | P0 |
| Cyclic PDO | shall | Domain + 预计算 Frame Plan | P0 |
| Multiple Tasks | may | 多 Domain 多速率 | P0 产品能力 |
| Online scan or ENI import | 至少一个 | 选择 online scan；cfggen 生成静态 C 配置 | P0 |
| Compare network | shall | 身份、位置、拓扑启动比较 | P0 |
| Explicit Identification | should | 支持并允许产品设为进入 OP 条件 | P1 |
| Station Alias | may | 支持 alias/position 匹配和配置寻址 | P0 产品能力 |
| EEPROM/SII read | shall | 异步 EEPROM FSM | P0 |
| Mailbox | shall | 固定请求池、异步收发 | P0 |
| Mailbox Resilient Layer | shall | 独立于 CoE 实现 | P0 |
| Mailbox polling | shall | PollTime 或 StatusBit | P0 |
| SDO normal/expedited | shall | upload/download + abort | P0 |
| Segmented SDO | should | 提升为 ESOP P0 | P0 |
| Complete Access | should；ENI import 时 shall | 首版 P1，不声明 ENI import | P1 |
| SDO Information | should | 可关闭诊断能力 | P1 |
| Emergency | shall | 固定事件记录并异步上报 | P0 |
| EoE/FoE/SoE | 条件要求 | 首版不声明；插件化 | P2 |
| BOOT | 固件下载时条件 shall | 实现 FoE/固件升级时一起交付 | P2 |
| DC | 声明支持时 shall | Motion FP 强制，因此完整实现 | P0 |
| Continuous propagation compensation | should | 周期/低频维护 slot | P1 |
| Sync window monitoring | should | 运动产品提升为 P0 | P0 |
| Slave-to-Slave via master | shall | 静态 copy plan + 源质量 | P0 |
| Master Object Dictionary | may | 非首发 | P2 |
| FP Motion: CiA 402 | mandatory | 独立 profile | P0 |
| FP Motion: DC | mandatory | 与 CiA 402 联合验收 | P0 |
| FP Motion: SERCOS profile | optional | 非首发 | P2 |

## 6. CiA 402 主站侧要求

### 6.1 主站与驱动的责任分界

驱动内部实现电流环、速度环、位置环和功率级保护。ESOP 主站侧负责：

- 发现驱动能力并验证 ESI、对象和 PDO mapping；
- 执行 PDS finite state automaton；
- 在正确状态和模式下发布目标值；
- 检查 Statusword、Modes display、实际值、错误码和质量；
- 处理 quick stop、disable、fault reset 和通信失效策略；
- 将厂商扩展封装在 profile compatibility/quirk 描述中。

### 6.2 驱动状态机

至少识别并测试以下状态：

| 状态 | 主站动作原则 |
| --- | --- |
| Not ready to switch on | 等待驱动自检；不得发送有效运动目标 |
| Switch on disabled | 可执行 Shutdown 序列，仍禁止运动 |
| Ready to switch on | 请求 Switch on |
| Switched on | 完成模式、参数和目标预置后请求 Enable operation |
| Operation enabled | 仅在 PDO、WKC、DC、命令 freshness 全部有效时更新目标 |
| Quick stop active | 执行配置的 quick-stop 行为，等待明确恢复决策 |
| Fault reaction active | 等待驱动完成内部故障反应，不重复乱发 reset |
| Fault | 记录错误，撤销运动使能；满足策略后以边沿语义执行 Fault reset |

状态识别必须采用 CiA 402 规定的 Statusword 位/掩码规则，由表驱动实现并进行穷举测试。不得以字符串、时序猜测或单个 bit 代替状态机。

### 6.3 最小对象集

| 对象 | 用途 | ESOP 要求 |
| --- | --- | --- |
| `0x6040` | Controlword | P0，RxPDO 优先，配置/诊断可 SDO |
| `0x6041` | Statusword | P0，TxPDO，状态机输入 |
| `0x6060` | Modes of operation | P0，映射或配置期 SDO，切换需确认 |
| `0x6061` | Modes of operation display | P0，必须确认实际模式 |
| `0x603F` | Error code | P0，故障诊断；可结合厂商错误历史 |
| `0x6502` | Supported drive modes | 能力存在时读取，不得假定所有驱动支持相同模式 |
| `0x607A` / `0x6064` | Target/actual position | CSP/位置模式使用 |
| `0x60FF` / `0x606C` | Target/actual velocity | CSV/速度模式使用 |
| `0x6071` / `0x6077` | Target/actual torque | CST/转矩模式使用 |
| `0x60C2` | Interpolation time period | 设备需要时配置并 read-back |
| `0x605A` 等 option code | Quick stop/shutdown/disable/halt 行为 | 按 ESI/驱动手册显式配置，不使用全局默认猜测 |
| 限位/误差对象 | torque、velocity、following error、software position limit | 按模式和设备 capability 绑定 |

表中的“使用”不表示所有对象在每个设备上都强制存在。`esop_cfggen` 必须依据 ESI、SDO Information、设备手册和 profile manifest 决定对象、访问权、PDO 可映射性、单位和缩放。

### 6.4 模式策略

| 模式 | 首发级别 | 关键条件 |
| --- | --- | --- |
| CSP | P0 | DC 锁定；周期 target position；实际位置和 following error 有效 |
| CSV | P0 | DC 锁定；target velocity、实际速度、限速和停机斜坡明确 |
| CST | P0 | DC 锁定；target torque、实际 torque、最大 torque 和失效归零策略明确 |
| Homing | P1 | homing method、速度、加速度、offset 和完成/错误 bit 按驱动验证 |
| Profile Position/Velocity/Torque | P1 | 非同步或设备内部 profile 场景；不得与 cyclic synchronous 模式混淆 |
| 厂商模式 | P2 | 独立 quirk/capability，不能污染通用状态机 |

### 6.5 模式切换与故障规则

1. 写入目标模式前，profile 根据设备能力决定是否必须先退出 Operation enabled。
2. 写 `0x6060` 后等待 `0x6061` 确认，超时或不一致立即失败。
3. 新模式的 PDO、单位、缩放、限制和控制参数必须已验证。
4. 切换第一周期必须使用可预测初值，避免位置跳变、速度突变或转矩阶跃。
5. WKC、DC、命令时效或驱动状态任一无效时，不发布“有效的新目标”。
6. Fault reset 是受策略控制的动作，不得在每周期持续置位。

### 6.6 安全边界

普通 EtherCAT、CoE 和 CiA 402 不构成功能安全通道。ESOP 可在通信故障时执行普通控制层的 hold、ramp-to-zero、quick stop 或 disable，但不能替代 STO、FSoE、安全 PLC、认证驱动参数和机器风险评估。

## 7. 持续无锁通信队列决策

### 7.1 当前是否已有

仓库已经以 Rust `no_std` 形式实现其中一部分固定容量交接；通用 DMA descriptor 所有权/缓存维护契约、Frame Plan 直接构建到 DMA TX descriptor、DMA TX 提交端口边界和 DMA RX descriptor 直接消费会话已经落地，但 MCU 具体 MAC 适配和跨任务 ProcBuf 原子发布仍只有设计，尚无目标板证据：

| 通道 | 设计机制 | 当前状态 |
| --- | --- | --- |
| supervisor -> RT command | 双页 snapshot + sequence release/acquire | 仅设计 |
| non-RT -> RT maintenance request | 固定容量 SPSC ring | 已实现通用 SPSC；维护请求类型待固化 |
| RT -> non-RT completion/event | 固定容量 SPSC ring + 固定诊断事件环 | 已实现通用 SPSC 和诊断事件环 |
| MAC DMA -> RT parser | descriptor ownership ring + cache/barrier | 已实现通用固定容量契约；具体 MAC/缓存属性适配待实板 |
| Domain input publish | staging 校验后按 generation/WKC 提交 | 已实现单 Domain 固定 staging/提交路径；跨任务页发布待实现 |
| RT control -> TX builder | 同一 RT owner 直接访问 | 无需队列 |

### 7.2 “持续无锁”的准确承诺

ESOP 只对满足单生产者/单消费者的通道采用 SPSC。该机制在初始化后连续跨周期工作，固定容量、无 heap、无 mutex、无 semaphore wait；但容量满时必须返回 `FULL/BUSY/DEFERRED`，不能覆盖未消费命令或无限自旋。

以下情况不允许误称为无锁：

- `_Atomic uint64_t` 在目标 ABI 上通过隐藏库锁实现；
- 多个 producer 同时写一个 SPSC ring；
- 关中断时间无上限；
- ring 满后 busy wait 直到 consumer 腾位置；
- 数据记录先发布 head/sequence，payload 后写完；
- cache clean/invalidate 或设备 barrier 缺失导致 DMA/CPU 看到旧数据。

### 7.3 队列实现需求

| ID | 优先级 | 需求 | 验收证据 |
| --- | --- | --- | --- |
| Q-001 | P0 | 提供固定容量、2 的幂容量的 typed SPSC ring；元素内存由调用方提供，初始化后零分配。 | host/ARM/RV32 build 和 allocator trap。 |
| Q-002 | P0 | producer 写 payload 后 release 发布 head；consumer acquire head 后读 payload；tail 反向同理。 | litmus test、TSAN model、反汇编审查。 |
| Q-003 | P0 | head/tail 使用目标平台可证明 lock-free 的自然字长；64 位序号不 lock-free 时使用 32 位 index + sequence snapshot。 | `atomic_is_lock_free`/编译器能力报告和目标机测试。 |
| Q-004 | P0 | `push/pop` 有界且不等待；满/空、drop、high-watermark 和 last sequence 可诊断。 | overflow/underflow/consumer stall 压力测试。 |
| Q-005 | P0 | 每个 ring 在类型和配置中固定 producer/consumer owner，禁止运行时多写者。 | 静态 API、并发 misuse 测试和文档检查。 |
| Q-006 | P0 | DMA ring 使用 descriptor owner、cache maintenance 和 device barrier，不以普通 C 原子替代硬件所有权协议。 | cache fault、DMA stress、descriptor wrap 测试。 |
| Q-007 | P0 | 事件可按声明策略覆盖最旧记录，命令/维护请求不得静默覆盖；所有丢弃可计数。 | ring 满策略矩阵测试。 |

## 8. 参考 IgH 的性能策略

ESOP 采用 IgH 已验证的 Domain、调用者驱动、显式 receive/process/queue/send、异步请求和 DC 预分配思想，并针对 MCU 做以下约束：

| 策略 | ESOP 决策 |
| --- | --- |
| 配置冻结 | 激活阶段完成映射、帧拆分、WKC、调度和预算；运行期计划只读 |
| 静态内存 | 单 arena、固定 slot/request/ring；激活后 heap 调用为 0 |
| RX 匹配 | 使用 256 项 index -> slot 表 O(1) 定位，再校验 generation/type/address/length |
| Frame Plan | 预计算每个 hyperperiod slot 的帧和数据报，周期内不遍历链表装箱 |
| DMA | TX descriptor 缓冲内构帧；RX staging 校验后提交；所有权和 cache 显式 |
| 数据完整性 | 半帧、旧帧或 WKC 错误不更新 committed input |
| 控制面隔离 | PDO/DC 为 P0，SDO/扫描为 P2；控制面同时受 byte/datagram/time 预算 |
| 多速率 | 多 Domain 周期和相位预计算，超限配置在激活时失败 |
| 诊断 | 热路径只写计数、时间戳和固定事件；文本/JSON/Protobuf 在非 RT 域 |
| 过载 | 先丢/延迟诊断和维护，再降低速 Domain；P0 超限进入明确 fault |

性能结论必须来自相同硬件、PHY、从站、PDO、周期、DC、线缆和控制面负载的测试。不能只因数据结构更简单就宣称普遍快于 IgH。

## 9. 建议模块和依赖

```text
esop_cfggen / ESI + product profile
             |
             v
esop_runtime
  +-- esop_wire
  +-- esop_engine
  +-- esop_scan + esop_sii + esop_al
  +-- esop_domain + esop_pdo
  +-- esop_mailbox + esop_coe
  +-- esop_dc
  +-- esop_profile_cia402
  +-- esop_diag
  +-- esop_queue / esop_procbuf
             |
             v
esop_port -> STM32 ETH | HPM GMAC | Linux raw/HIL
```

依赖规则：

- `wire/engine/domain/al` 不依赖 CiA 402；
- `cia402` 只依赖 CoE、Domain/ProcBuf 和诊断 API；
- queue 不依赖 OS 或 RTOS；
- port 不知道 CoE、PDO 或 CiA 402；
- cfggen 可使用宿主 XML 库，但生成结果不把 XML runtime 带入固件；
- ROS 2、Zenoh、Protobuf、文件系统和日志格式化不进入 RT 链接图。

## 10. 启动与周期行为

### 10.1 启动

```text
port qualification
  -> scan and identify
  -> compare static configuration
  -> INIT/PREOP
  -> mailbox/CoE parameterization
  -> PDO + SM/FMMU + watchdog
  -> DC initialization and lock
  -> SAFEOP, validate input/WKC
  -> CiA 402 profile ready
  -> OP
  -> per-axis CiA 402 enable sequence
```

任何一步失败都应保留具体 slave、对象、命令、expected/actual、AL/SDO code 和超时阶段。禁止只返回“启动失败”。

### 10.2 周期

```text
absolute timer release
  -> bounded RX DMA drain
  -> frame/datagram/WKC validation
  -> Domain input commit and quality
  -> acquire command snapshot
  -> CiA 402 state/mode/safety policy step
  -> prepare due PDO/DC/control slots
  -> build and submit TX DMA
  -> publish state/quality/event
```

单个 `esop_runtime_t` 只有一个 RT owner。非 RT 上下文只能通过 snapshot、SPSC ring 或只读诊断快照交互。

## 11. 验收与测试矩阵

### 11.1 协议功能

- 所有 EtherCAT command 的 golden frame、边界、错误长度和 WKC 测试；
- 1/8/32 从站扫描、地址、SII、身份和拓扑比较；
- Device Emulation、OpOnly、ESM timeout 和 AL error injection；
- SM/FMMU/watchdog/PDO 映射和 read-back；
- mailbox 丢帧、重复、counter wrap、PollTime、StatusBit；
- SDO expedited/normal/segmented、abort、Emergency；
- Slave-to-Slave copy 和源质量传播；
- DC delay/offset/start/drift/sync-window/失锁。

### 11.2 CiA 402

- 对 Statusword 输入做状态穷举和非法组合处理；
- 验证每条 Controlword transition、超时和禁止路径；
- CSP/CSV/CST 启动、停止、模式切换和初值连续性；
- WKC/DC/command age/drive fault 联合故障矩阵；
- 至少两种厂商驱动，记录对象、PDO、缩放和 quirk 差异；
- quick stop、fault reaction、fault reset 和驱动掉电恢复。

### 11.3 无锁与实时

- SPSC wrap-around、producer/consumer stall、满/空和 sequence gap；
- TSAN host model、ARM/RISC-V memory-order stress 和目标机长测；
- DMA/cache/descriptor ownership 压力测试；
- 30 分钟以上 1 ms/500 us HIL，deadline miss、正常 WKC mismatch、timeout 为 0；
- SDO/Emergency/诊断压力下 P0 周期 P99 回归不超过既定门槛；
- 激活后 malloc/free/new 次数为 0。

### 11.4 发布证据

每个发布候选至少生成：

- `capability_manifest.json`：Class/FP/协议/profile/平台能力；
- `robot_build_report.json`：内存、Flash、stack、frame、wire、copy 和配置 hash；
- `performance_report.json`：周期、jitter、P99/max、WKC、timeout、CPU 和错误计数；
- HIL topology manifest：设备顺序、ESI/SII identity、固件、PDO、线缆、PHY；
- conformance/interop report：内部测试、外部测试状态和未覆盖项。

## 12. 实施顺序与当前缺口

| 阶段 | 交付物 | 当前状态 |
| --- | --- | --- |
| M0 | `esop_queue`、wire codec、arena、Linux simulation port、测试框架 | 部分实现：wire codec、调用方固定 arena、固定帧池、SPSC ring、Linux AF_PACKET port、固定容量确定性 `SimulatedPort`、通用 DMA descriptor ownership/cache 契约和测试基础已具备；STM32/HPMicro 具体 DMA 端口仍未实现 |
| M1 | scan/SII/AL/SM/FMMU、单 Domain PDO、WKC、诊断 | 部分实现：scan/ESC、SII 身份读取、固定容量 EEPROM 分块读取、SyncManager/RxPDO/TxPDO category 只读解析、事务式固定容量配置候选、按 PDO 类别分段的多 SyncManager FMMU 逻辑地址分配、AL 单步转换、PDO 位域、SM/FMMU 校验与写入读回 FSM、固定容量 CoE PDO assignment/mapping 写序列、启动控制面闭环、单 Domain Frame Plan/WKC 提交、固定容量多 Domain/PDO/datagram 注册、SII segment datagram 绑定、MTU 拆帧与多速率激活编排、固定事件诊断和 Linux HIL 端口已具备；完整 SII/ESI 自动发现、真实从站 PDO 互操作和真实总线 HIL 未实现 |
| M2 | Mailbox resilient/polling、CoE SDO/Emergency、DC | 部分实现：固定容量 Mailbox 发送/轮询 FSM、有限预算重试、协议/计数器/长度异常恢复、可配置 Status Bit 轮询、CoE SDO expedited/segmented upload/download、abort、Emergency payload 解码及固定事件环接入、DC SYNC0/SYNC1 配置 FSM、FRMW reference-clock 周期同步槽、offset/jitter 锁定监测和主站控制请求闭环已具备；Status Bit 的 ESI/SII 自动发现、DC 拓扑传播延迟/全从站运行时同步和真实从站互操作仍未实现 |
| M3 | CiA 402 FSA、CSP/CSV/CST、两厂商驱动 HIL | 部分实现：独立 profile 已具备 Statusword FSA 解码、基础 Controlword 使能序列、生命周期拒绝、Fault reset 单脉冲、模式切换监督、实际模式确认、Operation Enabled 门槛和周期设定值首目标/限幅守卫；完整 PDO 对象绑定、厂商 quirk、三模式对象互操作和两厂商驱动 HIL 未实现 |
| M4 | STM32/HPM port、500 us 资格、完整 capability manifest | 未实现 |
| M5 | ETG 官方一致性/互操作流程、Class A 差距评估 | 未开始 |

实现优先级必须先保证 M0/M1 的协议正确性和无锁原语可证明，再叠加 CoE/DC/CiA 402。不能先写厂商伺服适配，再回头补通用 EtherCAT 状态机。

## 13. 官方参考

- [ETG.1500 EtherCAT Master Classes v1.0.2](https://www.ethercat.org/download/documents/ETG1500_V1i0i2_D_R_MasterClasses.pdf)
- [ETG Downloads: Implementation Directives](https://www.ethercat.org/en/downloads/downloads_956F888794FE4D428B98C3A4DBA8F303.htm)
- [EtherCAT Technology Group: Master implementation](https://www.ethercat.org/en/technology.html)
- [CiA 402 series: CANopen device profile for drives and motion control](https://www.can-cia.org/can-knowledge/cia-402-series-canopen-device-profile-for-drives-and-motion-control)
- [Beckhoff InfoSys: EtherCAT State Machine](https://infosys.beckhoff.com/content/1033/ethercatsystem/1036980875.html)
- [Beckhoff InfoSys: Working Counter](https://infosys.beckhoff.com/content/1033/ethercatsystem/1036996875.html)
- [Beckhoff InfoSys: Distributed Clocks](https://infosys.beckhoff.com/content/1033/ethercatsystem/2469118347.html)
- [Beckhoff InfoSys: CoE interface - parameter management](https://infosys.beckhoff.com/content/1033/ethercatsystem/2469073803.html)

规范全文的获取、复制、实现和商标使用受相应许可约束。本文是工程需求和决策摘要，不代替正式规范，也不是法律意见。
