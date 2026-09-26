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
真实从站固件一致。运行时激活证明静态期望与调用方提供的拓扑/ProcBuf
记录一致，但不等于真实 SII/PDO assignment read-back，也不证明驱动接受映射、
实际线缆时间、WCET、DMA/cache 正确性、制动/机械适配、STO/FSoE 或功能安全。生成示例和构建报告必须保持
`passed: false`，直到独立的目标构建、HIL、周期测量和发布审核提供证据。
