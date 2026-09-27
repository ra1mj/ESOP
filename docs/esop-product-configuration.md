# ESOP 产品配置生成器

`esop-cfggen` 是宿主机侧的确定性配置编译器。它读取严格的
`esop.product.v1` JSON 和显式选择的 ESI 设备/PDO 子集，复用运行时
`DomainRegistry`、`FramePlanSet`、`Cia402PdoMap`、轴策略校验和 ProcBuf
布局算法，生成不依赖 XML runtime 的静态产品证据。

## 1. 快速使用

仓库内的双轴驱动加 IO 示例可直接生成并校验：

```bash
make cfggen-example
make cfggen-runtime-example
make cfggen-build-report
```

等价的直接调用为：

```bash
cargo run -p esop-cfggen -- \
  --input config/examples/sim-dual-axis/product.json \
  --output build/generated/sim-dual-axis
```

生成成功后，严格 GCC 语法检查会验证静态 C header；运行时示例门会将
Rust 模块与检入黄金文件逐字节比较，并通过 `esop-product-config` 激活它。
CI 重新生成相同示例、校验产品化构建报告，并上传六个生成文件。

## 2. 输入契约

顶层 schema 固定为 `esop.product.v1`，未知字段、缺失字段和错误类型均
拒绝。产品清单显式包含：

- 产品名、robot ID 和策略版本；
- ProcBuf 轴数、IO 数、Domain 数和二次幂事件容量；
- 基准周期、deadline 和生成容量上限；
- 平台身份与可选 DMA 预算；
- Domain ID、逻辑地址、过程镜像范围、周期和相位；
- 从站位置、站地址、ESI 来源、vendor/product/revision/serial、PDO 选择；
- 每从站可选严格 `dc` 对象：`required` 表示必须确认 System Time 能力，
  `reference_clock` 表示该从站是唯一参考钟且隐含 `required=true`，`op_mode`
  为 DC-required 从站显式选择一个 ESI `Device/Dc/OpMode`；非 DC 从站不得选择模式；
- 每从站可选严格 `watchdog` 对象：`divider` 和 `process_data_intervals` 为
  独立可选的非零原始 `u16`；对象至少包含一项，缺失对象或字段表示保留对应
  ESC 默认值且不发写请求；
- CiA 402 轴、CSP/CSV/CST 模式、带方向的 SI/raw 缩放、机械范围和每周期限幅。

十六进制身份字段必须使用 `0x` 前缀。进入生成 C 字符串的名称/label 最长
128 UTF-8 bytes。ESI 路径必须相对产品清单，且 canonical path 不能经
`..` 或符号链接逃出清单目录。示例清单是该版本的可执行合同，不是任意
ESI/ENI 的兼容声明。

## 3. ESI 子集与校验

当前解析器支持 namespace-qualified XML 中的 vendor ID、Device Type
identity/name、四类 `StateMachine/Timeout`、带 `Enable`/`OpOnly` 的有序
SyncManager、`MBoxOut`/`MBoxIn` 的 `StartAddress`/`DefaultSize`/`ControlByte`、
`Mailbox/CoE`、有序且直接位于 `Device` 下的重复 `Fmmu` usage、
有序 `Device/Dc/OpMode`、RxPDO/TxPDO assignment，以及 byte-aligned PDO entry 的
index/subindex/bit length/DataType。未提供 timeout 时使用版本化的 ETG.1020
默认 profile。它明确拒绝：

- 模块化设备、嵌套/错位/未知或超过 16 项的 FMMU 声明，以及 FMMU 寄存器/逻辑地址规则；
- bit-packed 或嵌套 PDO entry；
- 零值、非法或纳秒换算溢出的 timeout，以及非 output SyncManager 的 `OpOnly`；
- 缺失 CoE、缺半边/重复/禁用的邮箱 SyncManager、缺失物理字段、容量越界及地址
  溢出或重叠；
- 未选择、重复、方向错误或宽度不匹配的对象；
- 缺少名称/AssignActivate、重复名称、数值宽度错误或包含非零直接
  `CycleTimeSync1` 的 DC OpMode；
- vendor-specific scaling、替代对象和隐式默认映射。

解析器同时保留 `MBoxOut`/`MBoxIn` 的 SyncManager index。若 ordered
`Fmmu` usage 声明 `MBoxState`，cfggen 必须从 `MBoxIn` index 推导直接
SyncManager 状态寄存器策略：地址为 `0x0800 + index * 8 + 5`、mask 为
`0x08`、active-high；不得从产品输入接受任意地址、mask 或极性。它还按从站
position 把该物理 bit 3 作为一位读 FMMU 打包到所属 Domain 的输入尾部，扩展
聚合 LRD 长度并把 input WKC 每映射增加一。生成绑定携带 Domain bit offset、
最大 age 和完整 FMMU 描述。没有该声明的设备继续生成 PollTime 策略。直接策略、
映射绑定和 mailbox SyncManager 证据都进入 JSON、inventory、C/Rust 和配置 hash。

生成前会一次性完成身份唯一性、Domain/过程镜像范围、静态容量、PDO
偏移、datagram、Frame Plan、多速率 schedule、expected WKC、CiA 402
对象集、轴掩码、SI/raw 可表示范围、ProcBuf layout 和周期预算校验。任何
失败都发生在发布输出目录之前。

## 4. 输出合同

| 文件 | 内容 |
| --- | --- |
| `esop_product_config.h` | 固定大小的 slave、Domain、PDO、datagram、轴策略和 ProcBuf 常量。 |
| `esop_product_config.rs` | 可直接编入 `no_std` 固件的静态产品合同与 32-byte 配置 hash。 |
| `product_config.json` | 规范化后的产品、注册、Frame Plan、schedule 和配置 hash。 |
| `device_inventory.json` | ESI identity、选择的 PDO 和语义化 ESI SHA-256。 |
| `procbuf_layout.json` | ProcBuf ABI v6、维度、精确字节数和 layout hash。 |
| `robot_build_input.json` | 设备数、PDO/frame/wire/WKC/copy、周期和资源输入。 |

生成的 inventory、JSON、C 和 Rust product slave 均携带精确主站发送/接收邮箱
地址与容量、显式 DC requirement/reference policy、所选 OpMode 的 SII 可表示描述符，
以及由产品基准周期解析出的绝对 SYNC0/SYNC1 周期、signed SYNC0 shift 和完整
16-bit `AssignActivate`；规范化 JSON、C 和 Rust 还携带 SII FMMU usage 实际数量、固定
16 项有序数组，以及 SyncManager 数量与 enabled mask，
ESI inventory 保留两侧 control byte。ESI mailbox、timeout profile、`OpOnly` mask 及
activation template、FMMU usage 顺序参与 ESI semantic hash 和配置 SHA-256；显式 DC policy 属于产品
语义，只参与配置 SHA-256，不反向改写 ESI 内容 hash。可选 watchdog 原始值同样属于
产品语义，并在 normalized JSON、inventory、C presence/value 字段和 Rust
`Option<EscWatchdogConfig>` 中保持精确一致。配置 SHA-256 只依赖规范化产品语义和排序后的 ESI 语义内容，不依赖 JSON
键顺序、XML 排版、输入/输出路径、主机或当前时间。同一语义输入必须生成
逐字节相同的六个文件。

输出先写入同级 staging 目录并同步文件，再以目录替换发布。失败保留原
输出；成功替换会删除旧 schema 遗留文件。

## 5. 固件运行时激活

`crates/esop-product-config/` 不解析 JSON、XML 或 C header，也不分配内存。
固件包含生成的 `esop_product_config.rs` 后，调用 `PRODUCT_CONFIG.activate`
并提供外部期望配置 hash、当前 boot ID、已扫描的 `SlaveRecord` 和实时
ProcBuf。激活按以下顺序 fail-closed：

1. 校验 `esop.product-runtime.v1` 与 32-byte 配置 hash；
2. 重算 ProcBuf ABI v6 layout，并校验 robot/boot/layout/region/capacity header；
3. 要求从站数量、position、station address、online、configured 和 identity 精确匹配；
4. 通过 `DomainRegistry` 重新登记 Domain/PDO/datagram，核对 PDO/datagram/WKC；
5. 通过既有 API 生成多速率 schedule 与每 Domain `FramePlanSet`；
6. 逐轴校验连续索引、驱动归属、冻结策略和选定模式的 `Cia402PdoMap`。

`PRODUCT_CONFIG.watchdog_plan()`、`dc_sync_plan()` 和 `startup_profiles()` 会在任何 Startup 动作发出前校验每个从站的
watchdog 对象非空、所有已配置值非零，并按生成从站顺序冻结 position、station address 和原始值；
完全未配置的产品得到空计划，不产生 ESC 请求。随后运行时校验每个从站的
DC reference 必须同时 required、全产品最多一个 reference、DC-required 从站必须且只有一个模式期望、非 DC 从站不得携带模式期望、timeout 非零、position 一一对应、生成邮箱范围有效、SII FMMU count 不超过 16、SyncManager count/enabled
mask 有界、`OpOnly` mask/flag 有效，并要求每个 `OpOnly` SyncManager 只关联所选
RxPDO、每个 PDO 分组连续。运行时还按所有 RxPDO group 后所有 TxPDO group 的确定性顺序
检查对应 FMMU usage 分别为 Outputs 和 Inputs，并从这些静态字段重新构建 schema-v2 SII 结构签名，而不是
信任预生成摘要。共享 `DcSyncTiming` resolver 还会按产品基准周期重新计算绝对寄存器值，
要求与生成值逐字段相等，并按产品从站顺序构造固定容量计划；非 DC 从站不得携带 timing，
唯一 reference 必须属于计划。DC booleans 分别映射为 `StartupDcRequirement::None`、`SystemTime`
或 `ReferenceClock`。`start_startup()` 将这些 profile 与精确
从站拓扑一起交给 `StartupController`。在线扫描完成后、任何身份读取之前，Startup 以
position-keyed 扫描证据验证所有 DC 要求：显式 reference 合格时选中它，否则选择扫描顺序
中的首个 System-Time-capable 从站；失败时不发布部分选择，也不进入 identity/SII/AL。
身份验证后、首个 AL 动作前，Startup 对携带
邮箱期望的 profile 精确读取 SII `0x001C..0x0020`，要求 CoE，并只比较 SII 可表示的
send/receive 地址和容量；轮询、超时、重试和 Status Bit 仍是运行期策略，不参与布局
相等判定。该布局先暂存而不发布。随后，携带 SII 期望的 profile 进入独立
`ReadingConfiguration` 阶段：从标准
`0x0040` 有界读取到 END，原子投影有序 FMMU usage 与 SM/RxPDO/TxPDO candidate，并比较
FMMU count/order/usage、SM 数量、enabled/OpOnly mask 以及按 Rx 后 Tx 冻结顺序排列的 PDO index、SM、object、subindex 和
bit length。Startup 还按生成的 `MBoxIn` index 精确比较对应在线 SM 的地址、容量、
control byte 与 enabled 状态，并要求 Status Bit 的存在性与 ordered
SyncManager-status FMMU usage 一致。相同完整镜像还会以借用方式解析 Strings `0x000a` 与 DC `0x003c`
固定 24-byte 条目，按精确模式名比较 cycle、shift、factor 与 AssignActivate。两类检查
全部成功后才同时按 position 发布完整 `MailboxConfig`、签名/DC 证据并开始 AL；在线 FMMU 条目数还必须不超过
扫描时 ESC 基础信息报告的 FMMU 数量。任何 stream、容量、
投影、动作所有权、deadline、结构或 DC 描述符差异均闭锁且不发布部分证据。未携带对应期望的 profile
和旧 `start()` API 保持原有兼容路径。
每个 ESM step 只建立一个绝对 deadline；相关 `OpOnly`
准备和 AL 请求/读回共享该 deadline。`StartupConfig.transition_timeout_ns != 0` 是
显式 legacy uniform override，并优先于生成 profile；零值选择逐转换 profile。
非 OP 及离开 OP 前必须禁用并读回所有 `OpOnly` 输出，进入 OP 时仅在 AL OP 已观测
后启用并读回，验证完成前不得进入 Ready。

任何阶段失败都不会返回部分运行时配置；成功结果持有只读 registry、
schedule、frame plan、轴模式、策略和 PDO map。每轴 PDO 临时映射上限固定为
32，cfggen 与运行时共享同一常量并在超限时拒绝。

配置期可进一步调用 `PRODUCT_CONFIG.build_pdo_startup_plan::<OPS>(position)`：
它按生成顺序为一个从站构建固定容量 CoE 计划，先清除 `0x1C10 + SM` 的
assignment count，再写该 SM 的 mapping 对象，最后发布 mapping index 列表和
count，并返回固定站地址。核心 `PdoConfigController` 对每个写值执行 download
和同对象 upload 回读，只有长度与字节完全一致才推进；分段 upload、代际、
动作、超时和 mismatch 均沿类型化故障路径 fail-closed。调用方可通过
`ScheduledPdoConfiguration` 把该控制器、`MailboxController` 和运行期
`MailboxConfig` 绑定到 `ScheduledProductionServiceScheduler`；配置屏障内调度器按
PDO Configuration、Watchdog Configuration、Mapping、DC Clock Configuration、DC SYNC Configuration、legacy DC Configuration、Startup 的固定顺序，
把每笔 CoE 请求交给现有邮箱/DC/共享 RX 路径，并在精确 upload 回读完成后才
放行 Configuration/CoE 生命周期门。调用方可在 `StartupConfig` 中冻结所需的
PDO Configuration、Watchdog Configuration、Mapping、DC Clock Configuration、DC SYNC Configuration 和 legacy DC Configuration 集合：所有期望从站先确认
PREOP，Startup 进入 `AwaitingConfiguration` 后只向这些服务让出优先级；调度器
仅在所有必需控制器真实进入 `Complete` 后释放屏障，并复用已验证从站表逐站经过
SAFEOP 到最终 SAFEOP/OP，不重新扫描或读取 SII。

`WatchdogController` 对计划中每个已配置字段先执行固定地址两字节小端写入，
再独立读取同一标准寄存器并精确比较：divider 使用 `0x0400/2`，process-data
intervals 使用 `0x0420/2`。每个动作要求 WKC 1、精确长度、generation/action/
control-pool 所有权和同一绝对配置 deadline；任一失败锁存首个类型化错误，
清空公开证据并阻止 PREOP 屏障释放。已被 ESC 接受的早期写入不声明回滚，
恢复必须显式重启控制器。

产品默认调用 `build_generated_pdo_configuration_batch::<JOBS, OPS>()`，直接使用每个
`ProductSlaveConfig` 中由 ESI 生成并校验的 `MailboxConfig`。构建器会从
`MBoxIn` descriptor 与 ordered FMMU usage 重新推导规范 Status Bit，并拒绝地址、
mask、极性、index 或声明存在性被篡改的配置；调用方不需要再调用
`with_status_bit`。原有 position-keyed
`ProductMailboxBinding` 与 `build_pdo_configuration_batch` 继续作为测试、维护和显式
替换路径。两条路径共用邮箱范围、重复站地址、job/operation 容量及逐站计划校验，
任一失败都不会返回部分批次。`PdoConfigBatch`
启动一次后复用同一 PDO/邮箱控制器，以 `base_generation + job_index` 自动推进；
空计划有界跳过，故障保留当前 index/station，只有显式重启才替换计划和清除故障。
`ScheduledPdoConfiguration::batch` 将当前 job 接入原有邮箱/DC/共享 RX 路径，报告
公开批 phase、当前 index、总 job 数和当前站地址。PDO 配置发生在 Mapping 生效前，
因此仍由直接 SM 状态字节抑制输入邮箱读取。产品激活会从 PDO 输入终点、Domain 周期、
从站顺序、MBoxIn index 和 ordered usage 重建 mapped binding，并拒绝 Domain、bit、age、
FMMU index、逻辑/物理地址、方向、覆盖范围或 LRD/WKC 被篡改。通过校验的绑定公开精确
`FmmuConfig` 给现有 Mapping 写入/读回 FSM；Mapping `Complete` 后，调用方显式启动
mapped mailbox，生产调度器只消费对应 Domain 的新鲜有效提交。inactive、invalid 或
stale 输入均抑制输入邮箱读取且不回退直接寄存器。无 `MBoxState` 声明的设备保持
PollTime，手工直接 Status Bit API 保持兼容。

核心另提供严格的 SII 标准邮箱五字固定头解析：只接受精确 word 起点/长度和已完成
`SiiBlockReader`，检查 CoE 协议位，并把从站 receive/send 字段转换为主站
send/receive `MailboxConfig`。Startup 已将该读取和生成 ESI 布局交叉验证接入身份与
AL 之间的有界控制请求路径。独立 `SiiCategoryStreamReader` 现可从标准 `0x0040`
开始，在一个绝对 deadline 内沿用同一 token/datagram 游标读取两字 header 和变长
payload，直到 END 才公开完整镜像；`SiiStreamDiscoveryController` 使用调用方 scratch
原子投影有序 FMMU usage 与 SyncManager/RxPDO/TxPDO candidate，并保留显式 signedness。
Startup 已复用同一流控制器和控制请求所有权，在首个 AL 动作前与生成产品重建的
schema-v2 固定大小结构签名
精确比较；signedness 暂不属于在线签名，因为 SII flags 的数据类型语义尚未单独冻结。
在线扫描现独立于生成 schema 精确采集 DC 端口接收时间和 Data Link Status，Startup
根据已有 DC requirement/reference policy 事务式发布固定容量拓扑与参考钟相对传播延迟；
产品必需 DC 从站缺少可测累计延迟时在 identity 前闭锁。`DcClockController` 可从该发布
拓扑构造固定容量计划，逐个精确读取 `0x0910/24`，以调用方应用时间样本和响应时单调时间
计算 32-bit 回绕或 64-bit 有符号 offset 修正，并把新 offset 与累计 delay 作为一个
`0x0920/12` 写入；参考钟 delay 为零，完整批次仅在所有精确 WKC 1 写入成功后发布，并可作为
独立必需服务接入上述 PREOP 屏障。其后 `DcSyncController` 先对所有计划从站关闭 `0x0981`，
逐站写入 `0x09a0` 的 SYNC0/SYNC1 周期对，只读取一次参考钟 `0x0910`，按所有重复周期的
checked LCM 选择严格未来的共同未移位 epoch，再把每站 signed shift 加到 `0x0990` start time，
最后逐站把完整 `AssignActivate` 写入 `0x0980`。软件证据仅在所有激活写入成功后原子发布；
已接受的硬件写入不会被描述为回滚。外部应用授时、周期性全从站漂移补偿、`0x092c`
sync-window、运行时锁定/恢复、物理响应真实性与时序精度和 HIL 仍待完成。

## 6. 构建报告接入

`generate-robot-build-report.py` 可选接收严格的产品输入：

```bash
make build-report \
  PRODUCT_INPUT=build/generated/sim-dual-axis/robot_build_input.json
```

脚本拒绝未知字段、无效 hash、零过程数据、deadline 超过周期和伪造的
`passed: true`。它把配置 hash、平台、设备、PDO/frame/wire/WKC/copy、周期和 ProcBuf
资源投影到 `esop.build.v1`；未传 `PRODUCT_INPUT` 时仍生成原有的主机
占位报告。

## 7. 资格边界

配置生成证明的是输入合同、静态布局和软件规划的一致性，不证明 ESI 与
真实从站固件一致。运行时可生成 PDO 和 ESC watchdog 配置计划，并分别对调用方交付的 SDO/寄存器响应做
逐字节 read-back 校验；生产调度器已通过确定性模拟端口覆盖邮箱发送、轮询、
跨周期请求所有权、重试/超时、精确回读、故障阻断和生命周期门控，但该软件
证据还覆盖全从站 PREOP 屏障、真实服务 phase 释放、保留拓扑、合法 SAFEOP/OP
顺序、逐转换 deadline 选择、`OpOnly` 写入读回顺序，以及调用方交付 SII 响应的邮箱
布局比对、完整 category stream/candidate 投影、ordered FMMU usage/schema-v2 生成结构签名
和 ESC count 门的 AL 前精确比较，以及调用方
交付 ESC/System Time 响应的能力判断和参考时钟选择；它不证明
该响应来自真实目标从站，也不等于真实从站 PDO
assignment/mapping、ESM timeout、SyncManager/FMMU 寄存器或 DC 时钟响应证据；模拟端口覆盖生成
双驱动计划的完整 watchdog 写入/读回和 DC SYNC 请求/RX/屏障路径，但不证明真实 ESC 定时行为、watchdog 实际周期或超时动作。该证据更不证明驱动
接受映射、完整周期 WKC、实际线缆时间、WCET、DMA/cache 正确性、制动/机械适配、
STO/FSoE 或功能安全。生成示例和构建报告必须保持
`passed: false`，直到独立的目标构建、HIL、周期测量和发布审核提供证据。
