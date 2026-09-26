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
- CiA 402 轴、CSP/CSV/CST 模式、带方向的 SI/raw 缩放、机械范围和每周期限幅。

十六进制身份字段必须使用 `0x` 前缀。进入生成 C 字符串的名称/label 最长
128 UTF-8 bytes。ESI 路径必须相对产品清单，且 canonical path 不能经
`..` 或符号链接逃出清单目录。示例清单是该版本的可执行合同，不是任意
ESI/ENI 的兼容声明。

## 3. ESI 子集与校验

当前解析器支持 namespace-qualified XML 中的 vendor ID、Device Type
identity/name、SyncManager、RxPDO/TxPDO assignment，以及 byte-aligned
PDO entry 的 index/subindex/bit length/DataType。它明确拒绝：

- 模块化设备和复杂 FMMU 规则；
- bit-packed 或嵌套 PDO entry；
- 未选择、重复、方向错误或宽度不匹配的对象；
- vendor-specific scaling、替代对象和隐式默认映射。

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

配置 SHA-256 只依赖规范化产品语义和排序后的 ESI 语义内容，不依赖 JSON
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
`MailboxConfig` 绑定到 `ScheduledProductionServiceScheduler`；调度器按
Startup、PDO Configuration、Mapping、DC Configuration、Mailbox 的固定顺序，
把每笔 CoE 请求交给现有邮箱/DC/共享 RX 路径，并在精确 upload 回读完成后才
放行 Configuration/CoE 生命周期门。调用方可在 `StartupConfig` 中冻结所需的
PDO Configuration、Mapping 和 DC Configuration 集合：所有期望从站先确认
PREOP，Startup 进入 `AwaitingConfiguration` 后只向这些服务让出优先级；调度器
仅在所有必需控制器真实进入 `Complete` 后释放屏障，并复用已验证从站表逐站经过
SAFEOP 到最终 SAFEOP/OP，不重新扫描或读取 SII。

产品调用方也可为每个 slave position 提供一个 `ProductMailboxBinding`，再调用
`build_pdo_configuration_batch::<JOBS, OPS>`。构建器会先校验绑定与产品从站一一
覆盖，再按冻结的产品顺序生成全部 job；缺失、重复或未知 position、重复站地址、
job/operation 容量不足及任一逐站计划错误都会在返回批次前失败。`PdoConfigBatch`
启动一次后复用同一 PDO/邮箱控制器，以 `base_generation + job_index` 自动推进；
空计划有界跳过，故障保留当前 index/station，只有显式重启才替换计划和清除故障。
`ScheduledPdoConfiguration::batch` 将当前 job 接入原有邮箱/DC/共享 RX 路径，报告
公开批 phase、当前 index、总 job 数和当前站地址。调用方仍须提供静态
`MailboxConfig`、SM/FMMU 与 DC 描述；本接口不发现这些硬件参数。

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
真实从站固件一致。运行时可生成 PDO 配置计划并对调用方交付的 SDO 响应做
逐字节 read-back 校验；生产调度器已通过确定性模拟端口覆盖邮箱发送、轮询、
跨周期请求所有权、重试/超时、精确回读、故障阻断和生命周期门控，但该软件
证据还覆盖全从站 PREOP 屏障、真实服务 phase 释放、保留拓扑及合法 SAFEOP/OP
顺序；它不等于真实从站 PDO assignment/mapping 或 AL 响应证据，也不证明驱动
接受映射、完整周期 WKC、实际线缆时间、WCET、DMA/cache 正确性、制动/机械适配、
STO/FSoE 或功能安全。生成示例和构建报告必须保持
`passed: false`，直到独立的目标构建、HIL、周期测量和发布审核提供证据。
