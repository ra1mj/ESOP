# ESOP 运动生命周期守卫设计

- 文档版本：1.0
- 日期：2026-09-03
- 状态：设计基线；HostObservation、固定转换审计、LifecycleSnapshot、ProcBuf lifecycle history 和 permit 拒绝审计已实现
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
| `exit_bad_cycles` | 从合格状态清除资格位所需的连续坏周期数；不延迟运动期的停止决定。 |
| `max_age_cycles` | 状态或输入可接受的最大年龄。 |
| `enter_threshold` / `exit_threshold` | 具有数值的门槛使用独立进入/退出阈值，形成滞回。 |
| `failure_class` | `HARD_LATCH`、`CONTROLLED_STOP`、`INHIBIT_ONLY`。 |
| `stop_action` | 对每轴或设备组请求的停止动作。 |

判定必须采用逻辑合取：所有适用必需门槛为 `GOOD` 才满足运动前提。不得以“90% 健康”或平均 WKC、平均时钟偏差替代必需项的逐项检查。

运动期任一必需门槛当周期为 `BAD` 或已经过期时，即使 `exit_bad_cycles` 尚未清除其资格位，仍立即进入 `STOPPING` 并拒绝新目标；退出计数只保留诊断滞回，不是继续运动的宽限期。同周期重复好值不得推进进入窗口，同周期坏值优先于好值，旧周期上报不得覆盖新观测。停止确认后重新进入运动仍需连续满足 `enter_good_cycles` 与新 permit；重新达到全部必需门槛后当前阻塞码清零，但历史转换仍保留原停止原因。

## 6. 故障分类与停止策略

### 6.1 故障分类

| 分类 | 示例 | 转换 | 后续恢复 |
| --- | --- | --- | --- |
| `HARD_LATCH` | deadline miss、DMA/cache 所有权违规、ABI/config hash 改变、拓扑身份改变、安全链 inhibit/未知、驱动 fault。 | `MOTION_ACTIVE -> STOPPING -> FAULT_LATCHED`。 | 全量重新资格检查、显式 recover、新 permit。 |
| `CONTROLLED_STOP` | command/permit/supervisor 超时、WKC 连续异常、DC 连续失锁、模式确认失败。 | `MOTION_ACTIVE -> STOPPING`。 | 停止完成且原因消失后回到 `READY`；重新 enable。 |
| `INHIBIT_ONLY` | 未收到 enable 请求、等待稳定窗口、维护待命、非运动设备不健康。 | 保持或转入 `READY`。 | 不需要 fault reset，但仍需要 permit 才能使能。 |

产品可将某项提高为更严格分类，不能在未重新评审的情况下将 `HARD_LATCH` 降低为 `CONTROLLED_STOP`。

当前 RT 守卫对**必需**门槛采用固定保守分类：平台、配置、拓扑、驱动、周期预算、外部安全观测失效为 `HARD_LATCH`，链路、Domain/WKC、DC、命令、supervisor 与 host observation 失效为 `CONTROLLED_STOP`。若多个门槛同周期失效，首个已记录阻塞码保持不变；最终锁存原因按外部安全、周期预算、驱动、配置、拓扑、平台的顺序选择。过期、缺失、未来周期或没有故障码的硬门槛用 `0x4741_0000 | (GateId + 1)` 标记不可用，不能仅因未显式上报 `false` 而降为受控停机。硬故障从运动中先请求停止，收到完整停机反馈后才锁存；恢复必须显式清故障并重新取得 permit。此分类是普通控制的 fail-closed 策略，不是对驱动 fault 或外部安全链的认证判定。

### 6.2 停止动作

停止动作由每轴 profile/产品配置决定，并必须与驱动文档、机械制动和系统安全设计一致：

| 动作 | 含义 | 使用约束 |
| --- | --- | --- |
| `HOLD` | 停止接受新的目标，保持最后受控目标。 | 仅限短暂、明确验证的控制策略；不是默认安全动作。 |
| `RAMP_TO_ZERO` | 以预配置斜坡将速度/转矩命令收敛到零。 | 只在数据质量仍足以控制时使用；超时后升级为 disable。 |
| `QUICK_STOP` | 由 CiA 402 option code 与驱动能力定义的受控停止。 | 必须按驱动/机器配置验证。 |
| `DISABLE` | 撤销普通运动使能，驱动离开 `Operation enabled`。 | 普通控制域的默认终态；不能替代 STO。 |

MLG 记录请求动作、实际驱动状态、停止开始/结束时间、超时和升级路径。若在 `stop_deadline` 内没有得到预期驱动状态，必须锁存故障。

当前固定容量 RT 实现支持在构造 `LifecycleGuard` 时通过 `AxisStopPolicy` 冻结最多 32 轴的动作；`cycle_axes` 对同一周期返回受守卫借用约束的逐轴决策，停机轴掩码保持为原已激活 permit 的轴集合，其他轴保持 inhibit。CiA 402 多轴适配器 `step_axis_bank` 对已停机轴分别输出 `QUICK_STOP` 或 `DISABLE`，维护模式与停止超时强制禁用普通运动使能；任何未授权轴都不能借旧的 Enable/FaultReset 请求启动。`HOLD` 和 `RAMP_TO_ZERO` 虽可在策略中表达，但当前尚无经过产品标定、输入质量验证和限幅的受控目标生成器；CiA 402 适配器将其降级为 `DISABLE`，不得把此降级称为已实现受控保持/减速。以上均为软件仿真验证，真实驱动、制动器和安全链的 HIL 仍是 FR-042 的待验收项。

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

实时域校验：boot ID 相同、策略版本已激活、source 被本地配置允许、authority 达到策略下限、epoch 单调不回退、轴掩码不为空、序号未重放且未超过 expiry。每次拒绝写入固定容量 `PermitAudit` 环，记录拒绝序号、单调时间、来源、epoch、permit 序号和拒绝原因；认证、签名、用户会话和 ACL 的复杂校验发生在网关或可信监督域，实时域不等待其结果。

RT 端 `GuardPolicy.allowed_axis_mask` 是随生命周期策略冻结的本地授权上限，`conservative()` 默认为 0（拒绝所有轴），产品激活前必须显式配置；网关的轴掩码校验只作入口预筛，不能代替 RT 的检查。`accept_permit` 拒绝任何包含本地未授权轴的 permit；在 `Active` 状态还要求续发 permit 的轴集合与本次已激活集合完全一致。扩轴或缩轴需先完成原集合的停机确认，再以新 permit 显式重新使能。拒绝尝试均记录在固定审计环，不覆盖仍有效的原 permit，也不消耗其序号；原 permit 到期或其他门槛失效仍按当前周期停止。当前已实现冻结的每轴独立停止策略，但真实驱动和 HIL 验证尚未完成。

### 7.3 恢复协议

恢复必须按以下顺序执行：

1. 停止动作完成，驱动确认处于允许恢复的非运动状态。
2. 原故障原因消失，所有适用门槛重新达到稳定窗口。
3. 授权实体提交一次显式 `recover_request`，其 `recovery_epoch` 大于已锁存的 epoch。
4. MLG 转入 `SYNCHRONIZING` 并重新验证配置、拓扑、时钟、Domain、驱动和外部 inhibit。
5. 监督域签发新的 permit；控制器再提交新的 enable request 与新鲜命令。
6. MLG 进入 `ENABLE_PENDING`，在每轴确认状态/模式/初值后才进入 `MOTION_ACTIVE`。

网络重连、旧命令重发、持续拉高 enable、持续发 fault reset、仅清除驱动 fault 或只重启上位机都不是恢复条件。

当前固定容量 `LifecycleGuard` 在进入和退出维护模式、请求锁存故障时撤销旧门槛资格；发生边界转换之前或同周期的门槛上报不得计入新的稳定窗口。运动中运行门槛或 permit 失效时立即撤销旧 permit，保存本次运动的轴掩码并请求配置的停止动作。从运动或停机中请求硬故障，先记录待锁存原因并保持 `STOPPING` 的配置停止动作，只有停机确认后才进入 `FAULT_LATCHED`；此前 `clear_fault` 和重新使能均被拒绝。重复上报不得覆盖待锁存原因；停止前已记录的首个阻塞原因与最终锁存原因分别保留。从非运动状态发起且无待确认停机的硬故障可直接锁存。已锁存时快照的停止动作为 `Disable`，与周期输出一致。

维护状态的周期动作显式请求 `Disable`，不得将普通 `Hold` 当作驱动断使能；从运动/停止中进入维护，退出后仍须维持 `Disable` 请求并显式确认停机，不能跳过停止确认。待锁存硬故障在维护模式切换期间不得丢失；已锁存故障不接受维护开关转换，避免绕过显式恢复。`cycle_axes` 只准备本周期动作；周期所有者必须在已验证停机 PDO 的帧成功提交到端口后，在该借用决策上调用 `mark_stop_transmitted`。构造帧、发送失败或只有周期决策均不能记为“已发出”；端口接受帧也不证明驱动已执行，仍需新鲜接收反馈。`acknowledge_stopped` 要求已经成功提交至少一次停机动作，且确认周期晚于该次提交、停机请求和本次 `STOPPING` 进入周期。`StopFeedback` 必须覆盖原 permit 的全部轴，且每轴同时有经验证的当前周期输入、静止判断和非 `Operation enabled` 判断；缺任一项、重复轴、错周期或旧周期证据均不能完成停机确认。可选的 `cia402` 适配器依据 Statusword 和实际速度生成单轴证据；缺速度 PDO、未知状态或驱动仍使能均 fail closed。速度阈值必须由产品结合驱动标定和机械设计确定，不能把 `Disable` 直接等同于机械静止。

从首次请求停机起，超过 `GuardPolicy.stop_timeout_cycles` 仍未确认时，即使维护模式切换或迟到的确认请求发生，也锁存 `STOP_TIMEOUT_FAULT_CODE` 并输出 `Disable`；首个阻塞原因仍单独保留。`clear_fault` 只有在故障后的必需门槛重新连续合格时才接受，并再次撤销旧 permit；恢复运动仍需显式 `request_rearm` 和更新 epoch 的 permit。维护退出也须由退出后的新观测重新满足稳定窗口，不能沿用维护前的健康计数。周期所有者必须从已完成帧校验、WKC 和输入年龄校验的 Domain 提供真实反馈；仿真测试不能替代实物驱动停机确认、产品安全链和 HIL 验证。

`verified_ethercat_stop_feedback` 在 `ethercat` 与 `cia402` 可选特性同时启用时，将原许可轴集合绑定到同周期 `CycleReport` 与 `Domain::finish_receive` 后的已提交输入。必须有完整、非零且符合预期的 WKC，无 RX 错误、超预算或丢帧，且停机控制动作在更早周期已由端口接受。无实际速度样本、驱动仍使能或状态不可辨时不能确认停稳。周期所有者仍需按先收上周期输入、评估并生成停机输出、成功提交下一帧并标记发送、再收新反馈、再调用 `acknowledge_stopped` 的顺序执行；模拟端口的失败发送和重试测试不代表真实设备验证。

可选 `ethercat` + `cia402` 的 `submit_stopping_frame` 为停机 TX 提供固定容量提交入口：周期所有者传入同一个可变借用的逐轴决策、由该决策生成的 CiA 402 输出、全部轴映射、预先核实非 CiA 402 输出均安全的冻结过程映像、Domain 与冻结的 FramePlan。发送前检查每轴停止控制字/模式字段位于匹配的可写 Domain 段、不同轴的输出字段不重叠，且后续可写 datagram 不会以不一致的过程映像偏移覆盖停机字段；无效控制字、只读或错地址计划、构帧失败和端口拒绝均不记 `issued_action`/首次发出周期，构帧失败释放未提交帧槽。只有端口接受完整帧后才在同一决策上记发送证据；调用方仍需按策略发布失败周期、重试或升级故障。该入口不负责周期调度、其他输出的安全校验、端口 DMA 所有权或实物停机证明，不能替代生产周期所有者与 HIL。

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

单调时间跨周期推进时，缺响应的 RX 索引可能仍处于 Armed。下次发帧前调用 `EthercatMaster::reap_expired_rx_before_tx`，仅在旧截止时间**之后**清理并记录 RX 超时；截止时间内继续拒绝相同索引的复用。该诊断属于旧响应，不应计作新周期已提交的数据。

### 8.2 ProcBuf 扩展

ProcBuf 应包含固定大小的 lifecycle 区域：

| 字段 | 写者 | 用途 |
| --- | --- | --- |
| `lifecycle_state` | RT | 当前 MLG 状态。 |
| `gate_required_mask` / `gate_good_mask` | RT | 必需门槛与当前合格门槛。 |
| `first_blocker_code` | RT | 当前或最近一次阻止运动的首个原因。 |
| `fault_latch_code` / `recovery_epoch` | RT | 锁存原因和最低有效恢复 epoch。 |
| `permit_epoch` / `permit_expiry_ns` | RT snapshot | 已绑定 permit 的审计摘要。 |
| `axis_stops[]` | RT | 每轴请求动作、实际发出的控制字动作及带周期标识的反馈证明位；不能将命令视为驱动已执行。 |
| `transition_seq` / `transition_time_ns` | RT | 用于事件与状态的因果关联。 |
| `transition_history[]` | RT State page | 固定容量、按时间顺序的最近状态转换、周期时间和故障码。 |

所有固定事件记录必须带 lifecycle state、gate/fault code、axis/device、transition sequence 和 monotonic timestamp。

ProcBuf ABI v4 在 State 页增加固定容量的 `axis_stops[AXES]`：`request_cycle` 标记当周期请求，`requested_action` 为策略动作，`issued_action` 仅在本周期停机帧成功提交端口后记录相应 CiA 402 控制字动作；发送失败或尚未提交为 0。对于缺乏受控目标发生器的 Hold/Ramp，成功提交后的 `issued_action=Disable`。只有从完成帧、WKC 与年龄合格且**晚于首次成功提交停机控制字的周期**的输入生成的 `StopFeedback` 才能填写 `feedback_cycle` 与 `feedback_valid/stationary/non_enabled`；首次提交的同周期输入不能冒充停机响应，缺失速度或未知驱动状态也不能证明已停稳。写入 API 校验决策、状态和反馈周期/轴掩码，并在失败时保持旧记录不变；下一周期无停止请求时清空旧证据。Protobuf v1 使用新增的可选 `LifecycleSummary.axis_stops` 字段承载同一语义，旧读者忽略该字段且经其重新编码会丢失。单值 `stop_action` 仍仅是旧接口的全局默认/摘要，**不是**逐轴实际反馈。需要停止旧 RT 与监督进程、重建共享区域，再以 v4 同版本重启；v1/v2/v3 header 均被拒绝。

停机超时在 `LifecycleGuard.stop_timeout_record()` 中保留超时周期、原轴掩码、首次发出停机的周期（可能缺失）、转换序号和升级当时每轴请求动作；进入 `FaultLatched` 后所有轴仍保持 inhibit，CiA 402 输出 Disable，但本周期 `axis_stops` 清空，因为没有新的可确认停机反馈。已提供 `stop_timeout_events_to_procbuf` 接口，供周期所有者将原轴集合逐轴写入 SPSC 事件环：`source=0x4D4C`、`code=1`、`severity=Fault`、`sequence=transition_sequence`、`value=0x53540001`、`axis_or_device=0` 起的轴号；`aux` 低两字节分别是 Protobuf 请求/发出动作（1..4），bit 16 表示此前确实发过停机控制字，高字节是 `FaultLatched` 状态值。事件时间戳是调用方提供的**写入时**单调时间，重试时可能晚于真实转换时间；应用可用相同的 boot ID 和转换序号关联 State 页的 `transition_cycle/time_ns` 和首个阻塞码。环满会返回错误并计入 `lost_events`，已写轴不重发，剩余轴下次调用再写；调用方应持续消费并重试，且避免在排空前覆盖旧超时记录。事件不证明机械静止，不改变既有 ABI 或 Protobuf 布局。完整生产周期所有者、受控 Hold/Ramp 与实物 HIL 仍待完成。

`LifecycleEventCursor::new(&guard)` 绑定启动实例，`lifecycle_events_to_procbuf` 先将守卫固定转换环中尚未发布的转换按序写入事件环，再尝试写入上面的逐轴超时事件；调用方只应选择这个组合接口或分别调用两个接口中的一个发布路径，避免事件乱序。转换事件使用 `source=0x4D4C`、`code=2`、`sequence=transition_sequence`、`axis_or_device=0xFFFF`、`value=transition.fault_code`，`aux` 低字节为原始 MLG `from_state`，第二字节为原始 MLG `to_state`。超时事件的高字节同样是原始 MLG 状态值，**不是** Protobuf `LifecycleState` 值（后者为未指定状态保留 0）；接收方应依据事件 code 分别解码。`FaultLatched` 事件为 Fault，Stopping/Maintenance 为 Warning，其余转换为 Info。环满时游标仅推进已写成功的转换，重试不会重复提交；若尚未写入的转换被 16 条历史覆盖，接口返回明确的 `HistoryOverrun { missed }`，调用方记录缺失量后才调用 `acknowledge_history_loss` 跳到最早可用转换。单调时间参数是实际写入时刻，不得冒充历史转换时刻；可选单 Domain 停机分支已在 State 成功发布后调用组合事件发布，其他周期分支仍须接线。

可选 `ethercat` + `cia402` + `procbuf` 的 `StopCycleContext` 是固定容量的**单 Domain 停机分支**，不是完整周期任务。输入必须是主站实际完成的当前 `CycleReport` 和 `Domain::finish_receive` 后的输入；调用前核对主站周期、State 序号、boot ID 与冻结许可轴数，不匹配时先拒绝、再避免修改门槛。它按顺序从真实 RX 投影门槛/质量，取得同一守卫决策和逐轴 CiA 402 输出，提交下一帧停机 PDO，连同失败 TX 的零发送证据写入 State，再只用更早成功提交之后的新鲜反馈确认停稳，最后发布 State 与有序事件。当前调用产生的转换使用本次单调时间；既有转换的时间须由调用者给出。状态页发送失败可由调用者保留重试，事件不会抢先于 State 发出。`NotStopping`（含超时锁存后的非停机状态）需要调用者明确路由至普通运动或禁止输出分支，并负责该分支的帧与状态/事件发布；调用者还负责最终 deadline 结果、其他 Domain、命令准入、非 CiA 402 输出安全映像与完整周期调度。模拟端口已覆盖失败 TX、随后成功提交、新鲜反馈确认和旧报告/轴容量不匹配拒绝；不能据此宣称完成产品周期闭环或实物 HIL。

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

`LifecycleGuard::snapshot(cycle, now_ns)` 提供发布所需的固定字段：当前状态、required/valid/qualified/ready 门槛位图、首个阻塞码、锁存故障码、permit epoch/expiry/current、转换序号/周期和恢复计数。`permit_audit_at` 以时间顺序读取固定容量的许可拒绝审计；ProcBuf 写者负责将快照映射到 `LifecycleSummary`，并将 `transition_at` 记录映射到 `LifecycleHistory` 的单调时间戳字段。

`CyclicQuality` 现在覆盖 platform、configuration/CoE、topology、DC、drive、Domain、WKC/link、command、supervisor、external safety 和 cycle budget；每项使用独立故障码投影到对应门槛，任一必需项失效都会按 MLG 策略阻止或停止运动。

可选 `esop-lifecycle-guard/ethercat` 将同周期的 `CycleReport`、**所有当周期必需且已调度** Domain 的 `finish_receive` 后质量，以及实际 `DcCyclicSync` 映射为 `CyclicQuality`。空 Domain 列表、过期/不完整 Domain、无新 DC 同步或 RX 错误均不判为健康；DC 门槛仍需链路未断、当周期成功同步且 monitor 锁定。平台、CoE、拓扑、驱动、命令、supervisor、外部安全和最终 deadline 由周期所有者从各自来源显式提供，不能从 EtherCAT 报告推断。`budget_exhausted` 与实际 deadline 分开判定。集成测试已验证 Linux 模拟端帧收发、Domain/DC 消费、WKC 故障、MLG 即时停止以及 ProcBuf v4 原始事实发布；它不是实物 HIL 或生产周期任务的证据。若配置不启用 DC，必须在冻结策略中显式豁免 DC 门槛，而非伪造已锁定状态。

同时启用 `ethercat` 和 `procbuf` 后，`ethercat_cycle_to_procbuf` 使用冻结调度的固定容量 Domain 顺序和当周期 due 掩码，从同一来源同时生成 MLG 原始事实与 ProcBuf 的 link/DC/每 Domain WKC 诊断；未调度 Domain 仍保留质量快照，但不会冒充当前周期通过，其输入年龄至少等于当前周期与最后成功周期的差。`dc_offset_ns` 是最后一次观测值，只有 `dc_locked=1` 才表示当前周期的锁定样本；AL 状态、命令年龄、累计 deadline miss 和故障位图仍由各自生产者填写。调用者仍须将返回的同一份事实交给 MLG，并在冻结调度中证明掩码与 Domain 清单一致。

多速率周期优先使用 `cyclic_quality_from_schedule` / `scheduled_ethercat_cycle_to_procbuf`：调度 tick 0 对应主站 `CycleReport.cycle=1`，无需调用方再传 due 掩码。每个固定槽以 `ScheduledDomainQuality` 显式绑定冻结的 Domain ID，槽数或 ID 不匹配即拒绝 Domain/WKC 门槛。已到期 Domain 需要本周期完整 WKC 和 `finish_receive` 成功；未到期 Domain 必须曾在最近一次**实际到期**的 tick 成功收包，且周期年龄小于其配置周期、快照仍完整有效。空到期 tick 可沿用上述有效输入，但 Link/WKC 仍需要本周期的有效接收证据（例如 DC 数据报），DC 门槛仍需要本周期同步。初次尚未成功接收、到期漏收、年龄/相位不符、RX 异常均保持 fail-closed。若没有本周期接收流量，不得将上一周期的 WKC 当成当前 WKC。旧的手动 due 掩码入口保留兼容语义，空 due 掩码不算健康。

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
