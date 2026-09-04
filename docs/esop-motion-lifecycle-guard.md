# ESOP 运动生命周期守卫设计

- 文档版本：1.0
- 日期：2026-09-03
- 状态：设计基线；HostObservation 运行时契约已实现
- 上游需求：[ESOP 软件产品需求文档](esop-software-prd.md) FR-039 至 FR-046、NFR-017

## 1. 目的与安全边界

运动生命周期守卫（Motion Lifecycle Guard，`MLG`）是 ESOP 实时域中的 fail-closed 控制机制。它持续检测系统是否具备开启和保持普通运动控制的前提，并对 EtherCAT/CiA 402 的运动使能进行唯一的生命周期裁决。

MLG 的目标是防止以下情况导致运动继续或重新开启：

1. 平台、配置、拓扑、时钟或过程数据尚未完成资格检查。
2. EtherCAT、DC、驱动、实时执行或外部监督状态在运动期间失效。
3. 命令属于旧启动实例、已过期、序号重放、越过授权轴范围或未经过恢复流程。
4. 安全链路的观测输入表明外部 inhibit 已激活或输入不可用。
5. 拓扑、配置、固件或维护动作改变了已验证的运行前提。

MLG 是普通控制域的风险控制机制，**不是**认证功能安全组件。它不替代 STO、FSoE、安全 PLC、安全继电器、双通道急停、机械限位、驱动安全参数或机器风险评估。任何包含人身安全目标的产品必须将这些功能设计、实现、验证和认证为独立安全链路。MLG 可以读取安全链状态并撤销普通运动许可，但不能据此宣称安全完整性等级。

当前实现已提供 `HostObservation` 固定结构和独立门槛，用于接收 Linux/eBPF 监督域的带 `boot_id`、`agent_epoch`、`heartbeat_seq`、单调时间、观测状态和丢失计数的证据。它只能影响门槛资格，不能直接授权运动、修改 controlword 或清除故障锁存。

## 2. 设计目标与非目标

### 2.1 目标

1. 运动必须以明确且当前的准入证据开启，并在证据丢失时 fail closed。
2. 每个运动门槛独立可见、可追溯、可配置；禁止以综合健康分掩盖单点失效。
3. 整个判定在固定时间和固定内存内完成，适合 1 ms 和 500 us 周期。
4. 状态迁移、停止动作、故障锁存与恢复前提均可从 ProcBuf 和事件记录还原。
5. 外部网络、ROS 2、Zenoh、Protobuf、磁盘或日志系统不可阻塞 MLG 决策。

### 2.2 非目标

1. 不创建第二条认证安全通道，不计算 SIL/PL，也不替代现有安全控制器。
2. 不决定机器级风险阈值、碰撞限制或制动距离；这些由产品安全与控制策略定义。
3. 不直接处理 TLS、签名验证或远程用户认证。网关/监督域验证身份，实时域只校验固定格式的本地 motion permit。
4. 不允许多控制器直接仲裁同一轴。控制仲裁在监督域或实时应用策略中完成，MLG 只检查其结果是否有效。

## 3. 架构位置与所有权

```text
network / ROS 2 / Zenoh / UI
        -> gateway authentication and policy
        -> fixed IPC command + motion permit
------------------- real-time boundary -------------------
ProcBuf command snapshot + EtherCAT/drive/clock/platform snapshots
        -> MLG evaluate()              (single RT owner)
        -> per-axis stop/enable policy (single RT owner)
        -> CiA 402 profile step
        -> PDO output / ProcBuf lifecycle state / event ring
```

MLG 的唯一写者是实时任务。网关、ROS 2 控制器、维护服务和测试工具只能写入受限的 Command/permit 输入或读取状态；它们不能直接修改 MLG 状态、覆盖 fault latch、写 CiA 402 enable controlword 或跳过停止动作。

## 4. 生命周期状态机

### 4.1 状态定义

| 状态 | 含义 | 运动输出规则 |
| --- | --- | --- |
| `BOOT` | 上电或 RT runtime 未初始化。 | 不发布有效目标；驱动不得请求 `Operation enabled`。 |
| `QUALIFYING_PLATFORM` | 验证 port、时钟、DMA/cache、ProcBuf ABI 与静态资源。 | 禁止运动。 |
| `DISCOVERING` | 扫描 EtherCAT 网络并读取身份/能力。 | 禁止运动。 |
| `CONFIGURING` | 执行 AL、SM/FMMU、PDO、CoE 和 profile 配置。 | 禁止运动。 |
| `SYNCHRONIZING` | 等待 Domain/WKC、DC、驱动状态和输入稳定。 | 禁止运动。 |
| `READY` | 运行前提稳定，但没有有效 motion permit 或明确 enable 请求。 | 禁止运动，驱动保持非运动使能状态。 |
| `ENABLE_PENDING` | 已收到 permit/enable 请求，正在执行各轴模式、初值和 CiA 402 使能序列。 | 仅执行必要的非运动状态转换；不发布普通运动目标。 |
| `MOTION_ACTIVE` | 全部门槛与 permit 当前有效，允许发布受限目标。 | 仅按 per-axis policy 发布目标。 |
| `STOPPING` | 检测到运行期前提失效，正执行每轴停止策略并等待状态确认。 | 拒绝新目标；执行 hold、ramp-to-zero、quick stop 或 disable。 |
| `FAULT_LATCHED` | 不可自动恢复的失败或停止未完成。 | 禁止运动，需显式恢复。 |
| `MAINTENANCE` | 重配置、升级或维护请求已获准。 | 禁止运动，驱动不得保持 `Operation enabled`。 |
| `SHUTDOWN` | 正在去激活或断电。 | 禁止运动。 |

`READY` 不表示机械安全，也不表示驱动已经 `Operation enabled`。它只表示 ESOP 的普通控制前提已稳定，仍需有效 permit 和受控使能流程。

### 4.2 允许的主路径

```text
BOOT
  -> QUALIFYING_PLATFORM
  -> DISCOVERING
  -> CONFIGURING
  -> SYNCHRONIZING
  -> READY
  -> ENABLE_PENDING
  -> MOTION_ACTIVE
```

从任何非运动状态，配置、拓扑、平台或安全链硬失败可进入 `FAULT_LATCHED`。从 `MOTION_ACTIVE` 发现任何运行期门槛失效，必须先进入 `STOPPING`；停止确认后进入 `READY` 或 `FAULT_LATCHED`，由故障分类和恢复条件决定。任何维护请求都先撤销 permit 并进入 `MAINTENANCE`。

### 4.3 转换规则

| 来源 | 触发条件 | 目标 | 必要动作 |
| --- | --- | --- | --- |
| `BOOT` | 静态内存、ABI、端口初检成功 | `QUALIFYING_PLATFORM` | 初始化门槛记录和 fault latch。 |
| `QUALIFYING_PLATFORM` | 平台资格有效 | `DISCOVERING` | 启动扫描。 |
| `DISCOVERING` | 拓扑与身份匹配 | `CONFIGURING` | 冻结发现快照。 |
| `CONFIGURING` | 配置 read-back、AL/SM/FMMU/PDO/profile 成功 | `SYNCHRONIZING` | 激活周期与时间同步检查。 |
| `SYNCHRONIZING` | 所有运行前门槛连续有效 | `READY` | 撤销所有旧 permit；驱动保持非使能。 |
| `READY` | permit、enable request、轴初值和 CiA 402 使能前提有效 | `ENABLE_PENDING` | 绑定 permit epoch，开始受控使能。 |
| `ENABLE_PENDING` | 所有申请轴确认 `Operation enabled` 且门槛仍有效 | `MOTION_ACTIVE` | 发布第一个受限目标。 |
| `MOTION_ACTIVE` | 任一运行门槛失效 | `STOPPING` | 记录首个触发原因，撤销 permit，执行停止策略。 |
| `STOPPING` | 所有申请轴确认停止 | `READY` 或 `FAULT_LATCHED` | 可恢复原因回到 `READY`；硬失败或超时锁存。 |
| `FAULT_LATCHED` | 原因消失、稳定窗口满足、显式 recover 和新 permit | `SYNCHRONIZING` | 重新验证，禁止直接跳到 `MOTION_ACTIVE`。 |
| 任意非 `MOTION_ACTIVE` | 维护请求获准 | `MAINTENANCE` | 清 permit，确保驱动非使能。 |
| `MAINTENANCE` | 维护结束 | `QUALIFYING_PLATFORM` | 全量重新资格检查。 |

状态机不允许以下转换：`FAULT_LATCHED -> MOTION_ACTIVE`、`MAINTENANCE -> MOTION_ACTIVE`、`READY -> MOTION_ACTIVE`、网络重连后自动进入 `MOTION_ACTIVE`、只写 CiA 402 fault reset 后恢复运动。

## 5. 门槛模型

### 5.1 门槛类别

MLG 每周期读取一个固定大小的 `lifecycle_evidence` 快照。每项门槛均有 `GOOD`、`BAD`、`UNKNOWN`、`STALE` 状态，且 `UNKNOWN` 与 `STALE` 均不允许运动。

| 门槛组 | 必需证据 | 启动期要求 | 运动期要求 | 默认失效处理 |
| --- | --- | --- | --- | --- |
| `platform` | RT tick、DMA ownership/cache、资源冻结、ProcBuf ABI。 | 必须通过。 | deadline/DMA 故障即失败。 | `FAULT_LATCHED`。 |
| `configuration` | config/layout hash、设备清单、策略版本。 | 必须匹配。 | 任一变化即失败。 | `FAULT_LATCHED`。 |
| `topology` | 位置、identity、AL 状态、链路。 | 必须匹配且进入要求状态。 | 所有 P0 设备保持合格。 | `STOPPING` 后锁存。 |
| `domain` | 必需 Domain 的 WKC、input age、frame 完整性。 | 连续有效。 | 当周期和连续健康要求均满足。 | `STOPPING`。 |
| `clock` | DC lock、offset/jitter、应用单调时间。 | 若 profile 要求 DC，则连续锁定。 | 维持配置阈值。 | `STOPPING`。 |
| `drive` | CiA 402 state、mode display、fault/following error、实际值质量。 | 必须 ready。 | 申请轴必须处于期望状态。 | `STOPPING` 或 `FAULT_LATCHED`。 |
| `command` | boot ID、sequence、deadline、轴掩码、限幅与模式。 | 不要求有效命令。 | 必须当前有效。 | `STOPPING`。 |
| `permit` | 来源、permit epoch、策略版本、expiry、恢复 epoch。 | 不要求。 | 必须当前有效。 | `STOPPING`。 |
| `supervisor` | 实时域可测的 IPC heartbeat。 | 仅 split 部署时要求。 | 配置的失联窗口内有效。 | `STOPPING`。 |
| `external_inhibit` | 安全链/使能链的只读观测输入。 | 需要明确为 clear。 | 必须持续 clear。 | `STOPPING`；输入缺失/未知为失败。 |

一个产品配置可以声明某些门槛不适用，例如无 DC 从站的 IO-only profile，但该豁免必须在生成配置和 capability manifest 中显式出现。禁止在运行时把必需门槛降级为可选。

### 5.2 稳定性、时效和滞回

每个门槛配置以下参数：

| 参数 | 含义 |
| --- | --- |
| `enter_good_cycles` | 从不合格进入合格前必须连续满足的周期数。 |
| `exit_bad_cycles` | 运行期从合格进入失败所需的连续坏周期数；硬失败可设为 1。 |
| `max_age_cycles` | 状态或输入可接受的最大年龄。 |
| `enter_threshold` / `exit_threshold` | 具有数值的门槛使用独立进入/退出阈值，形成滞回。 |
| `failure_class` | `HARD_LATCH`、`CONTROLLED_STOP`、`INHIBIT_ONLY`。 |
| `stop_action` | 对每轴或设备组请求的停止动作。 |

判定必须采用逻辑合取：所有适用必需门槛为 `GOOD` 才满足运动前提。不得以“90% 健康”或平均 WKC、平均时钟偏差替代必需项的逐项检查。

## 6. 故障分类与停止策略

### 6.1 故障分类

| 分类 | 示例 | 转换 | 后续恢复 |
| --- | --- | --- | --- |
| `HARD_LATCH` | deadline miss、DMA/cache 所有权违规、ABI/config hash 改变、拓扑身份改变、安全链 inhibit/未知、驱动 fault。 | `MOTION_ACTIVE -> STOPPING -> FAULT_LATCHED`。 | 全量重新资格检查、显式 recover、新 permit。 |
| `CONTROLLED_STOP` | command/permit/supervisor 超时、WKC 连续异常、DC 连续失锁、模式确认失败。 | `MOTION_ACTIVE -> STOPPING`。 | 停止完成且原因消失后回到 `READY`；重新 enable。 |
| `INHIBIT_ONLY` | 未收到 enable 请求、等待稳定窗口、维护待命、非运动设备不健康。 | 保持或转入 `READY`。 | 不需要 fault reset，但仍需要 permit 才能使能。 |

产品可将某项提高为更严格分类，不能在未重新评审的情况下将 `HARD_LATCH` 降低为 `CONTROLLED_STOP`。

### 6.2 停止动作

停止动作由每轴 profile/产品配置决定，并必须与驱动文档、机械制动和系统安全设计一致：

| 动作 | 含义 | 使用约束 |
| --- | --- | --- |
| `HOLD` | 停止接受新的目标，保持最后受控目标。 | 仅限短暂、明确验证的控制策略；不是默认安全动作。 |
| `RAMP_TO_ZERO` | 以预配置斜坡将速度/转矩命令收敛到零。 | 只在数据质量仍足以控制时使用；超时后升级为 disable。 |
| `QUICK_STOP` | 由 CiA 402 option code 与驱动能力定义的受控停止。 | 必须按驱动/机器配置验证。 |
| `DISABLE` | 撤销普通运动使能，驱动离开 `Operation enabled`。 | 普通控制域的默认终态；不能替代 STO。 |

MLG 记录请求动作、实际驱动状态、停止开始/结束时间、超时和升级路径。若在 `stop_deadline` 内没有得到预期驱动状态，必须锁存故障。

## 7. Motion Permit 与恢复协议

### 7.1 Permit 目的

普通命令只表达“期望控制什么”；motion permit 表达“当前启动实例、策略和控制权允许哪些轴进入普通运动”。二者必须同时有效，且均由实时域以固定大小、无等待方式校验。

### 7.2 固定契约

`motion_permit` 至少包含：

```text
robot_id
boot_id
source_id
permit_epoch
recovery_epoch
policy_version
axis_mask
command_sequence_floor
issued_monotonic_ns
expires_monotonic_ns
```

实时域校验：robot/boot ID 相同、策略版本已激活、source 被本地配置允许、epoch 单调不回退、轴掩码不越权、命令序号未重放、未超过 expiry、permit 未在当前故障后失效。认证、签名、用户会话和 ACL 的复杂校验发生在网关或可信监督域；实时域不等待其结果。

### 7.3 恢复协议

恢复必须按以下顺序执行：

1. 停止动作完成，驱动确认处于允许恢复的非运动状态。
2. 原故障原因消失，所有适用门槛重新达到稳定窗口。
3. 授权实体提交一次显式 `recover_request`，其 `recovery_epoch` 大于已锁存的 epoch。
4. MLG 转入 `SYNCHRONIZING` 并重新验证配置、拓扑、时钟、Domain、驱动和外部 inhibit。
5. 监督域签发新的 permit；控制器再提交新的 enable request 与新鲜命令。
6. MLG 进入 `ENABLE_PENDING`，在每轴确认状态/模式/初值后才进入 `MOTION_ACTIVE`。

网络重连、旧命令重发、持续拉高 enable、持续发 fault reset、仅清除驱动 fault 或只重启上位机都不是恢复条件。

## 8. 实时执行与数据契约

### 8.1 每周期顺序

MLG 的每周期执行顺序固定如下：

```text
1. receive/validate/commit EtherCAT inputs
2. acquire ProcBuf command + permit snapshot
3. collect fixed lifecycle evidence
4. update per-gate debounce, age and first-failure record
5. evaluate MLG state transition
6. apply per-axis stop/enable constraint to CiA 402/profile step
7. prepare/send PDO/DC
8. publish State, Quality, Lifecycle and events
```

MLG 不读取网络、不运行 SDO、不分配内存、不格式化日志，也不从 ISR 调用。`evaluate()` 使用连续数组、固定枚举和有界循环；所有时间均来自 ESOP 单调时间，不能以 ROS time 或 wall clock 判断 permit 时效。

### 8.2 ProcBuf 扩展

ProcBuf 应包含固定大小的 lifecycle 区域：

| 字段 | 写者 | 用途 |
| --- | --- | --- |
| `lifecycle_state` | RT | 当前 MLG 状态。 |
| `gate_required_mask` / `gate_good_mask` | RT | 必需门槛与当前合格门槛。 |
| `first_blocker_code` | RT | 当前或最近一次阻止运动的首个原因。 |
| `fault_latch_code` / `recovery_epoch` | RT | 锁存原因和最低有效恢复 epoch。 |
| `permit_epoch` / `permit_expiry_ns` | RT snapshot | 已绑定 permit 的审计摘要。 |
| `stop_action_requested` / `stop_action_observed` | RT | 每轴或设备组停止请求与结果。 |
| `transition_seq` / `transition_time_ns` | RT | 用于事件与状态的因果关联。 |

所有固定事件记录必须带 lifecycle state、gate/fault code、axis/device、transition sequence 和 monotonic timestamp。

## 9. 配置与可观测性

### 9.1 生成配置

每个产品配置应生成：

1. 适用门槛、稳定窗口、年龄、阈值、故障分类和停止策略。
2. 每轴默认和升级停止动作、停止超时、允许的模式与 enable 顺序。
3. 允许的 source、axis mask、permit 策略版本和 supervisor heartbeat 条件。
4. 对 DC、外部 inhibit、非 EtherCAT 设备和 maintenance 的适用性声明。
5. `lifecycle_policy_hash`，并将其写入 ProcBuf、capability manifest、build report 和 performance report。

MLG 配置与 EtherCAT/ProcBuf 配置一起冻结。变更 policy hash、门槛、停止动作、轴权限或安全链观测映射都要求重新资格检查，不能热更新到 `MOTION_ACTIVE` 系统。

### 9.2 诊断输出

诊断至少包括：

1. 当前/前一 lifecycle state、状态转换次数和持续时间。
2. 每个门槛的状态、连续好/坏周期、年龄、阈值和最近变化时间。
3. 首个阻塞原因、所有并发失败原因、fault latch、停止策略与升级原因。
4. permit 来源、epoch、轴掩码、expiry、拒绝原因和 replay 计数。
5. enable/stop 命令与 CiA 402 实际状态的时间关联。

实时域只更新结构化计数与固定事件。文本、JSON、Protobuf、告警通知和历史记录由非实时域从快照异步生成。

## 10. 验收与验证

| ID | 类型 | 验收要求 |
| --- | --- | --- |
| MLG-001 | 状态机 | 穷举所有状态与允许转换；禁止转换必须被拒绝并记录。 |
| MLG-002 | 门槛组合 | 属性测试证明：任意必需门槛为 `BAD`、`UNKNOWN` 或 `STALE` 时，无法进入或保持 `MOTION_ACTIVE`。 |
| MLG-003 | 抖动与时效 | 对每种门槛验证连续周期、滞回、age、短暂恢复和长时间失效行为。 |
| MLG-004 | 停止动作 | 对 hold、ramp、quick stop、disable 验证请求、驱动反馈、超时和升级路径。 |
| MLG-005 | 锁存/恢复 | 验证硬故障不能通过重连、旧 permit、持续 enable 或持续 fault reset 自动恢复。 |
| MLG-006 | Permit | 验证 boot ID、source、policy version、epoch、轴权限、sequence、TTL 和 replay 拒绝。 |
| MLG-007 | HIL | 在双轴和 IO 拓扑上注入 WKC、DC、AL、驱动 fault、链路、命令、supervisor、deadline 与外部 inhibit 故障。 |
| MLG-008 | 实时性 | 1 ms/500 us Q1/Q2 中测量 MLG P99/max、零分配、无等待，且不改变 P0 frame plan。 |
| MLG-009 | 可追溯性 | ProcBuf、event ring、performance report 和 HIL trace 对同一 transition sequence 给出一致结论。 |
| MLG-010 | 配置变更 | topology/config/policy hash/maintenance 变更后，旧 permit 无效且必须全量重新资格检查。 |

## 11. 交付阶段

| 阶段 | 交付物 | 门槛 |
| --- | --- | --- |
| MLG-R0 | 状态表、门槛词典、ProcBuf/permit schema、生成配置、host 状态机模型。 | MLG-001 至 MLG-003 通过。 |
| MLG-R1 | RT `evaluate()`、事件、基本 stop/disable、CiA 402 约束。 | MLG-004、MLG-008 通过。 |
| MLG-R2 | DC/WKC/驱动/命令/外部 inhibit HIL，fault latch/recovery。 | MLG-005、MLG-007、MLG-009 通过。 |
| MLG-R3 | gateway permit、ACL 审计、ROS 2/Zenoh 状态映射。 | MLG-006、MLG-010 和安全边界评审通过。 |

## 12. 未决决策

1. 外部 inhibit 的实际来源、信号极性、诊断覆盖率和其与认证安全链的接口责任。
2. 每轴/产品的默认停止动作、斜坡、quick stop option code、停止超时和机械制动时序。
3. permit 的可信签发域、密钥/硬件信任边界、source identity 方案和离线维护策略。
4. supervisor heartbeat 的最大允许年龄、无监督本地控制是否允许以及其轴范围。
5. 哪些 DC、following error、温度、电源和外设质量条件应升级为 `HARD_LATCH`。
6. 功能安全项目的适用标准、系统安全负责人和独立验证计划。
