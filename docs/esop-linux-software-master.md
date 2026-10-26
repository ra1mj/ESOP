# Linux 软件主站运行与验收

本交付按原始软件 PRD 的 R0–R2/P0 建立 Linux 软件闭环。单片机端和真实
硬件资格按本次范围延后；软件场景成功与硬件 `qualified` 使用不同结论。

## 运行

在仓库根目录执行：

```bash
cargo run -p esop-ethercat-linux-port --example r0_r2 -- --help
cargo run --release -p esop-ethercat-linux-port --example r0_r2 -- \
  --backend sim --scenario full --mode csp --cycles 10000 \
  --output-dir build/r0-r2/sim
```

默认产品是两个独立 CiA 402 轴加一个 DI/DO 从站，1 ms 虚拟周期。
未知场景会在打开端口前拒绝；`q1`、`q2`、`q4`、`q4-baseline` 会规范化为表中名称，
捕获、运行记录和离线导入使用相同场景标识。
`--mode csv`、`--mode cst` 选择各自冻结的生成配置。程序执行真实 EtherCAT
字节流、SII/寄存器/PDO 回读、生命周期许可、停止和新 permit 恢复，不向主站
注入完成状态。虚拟 ESC 独立计算寻址、SM/FMMU 访问和 WKC。
AL/DL 监测的时间界限会随周期所有者传递，并在运动输出前重新检查；已经接受的
报文记录保持真实，随后发现过期时按生命周期策略请求停止。

| 场景 | 轴/从站 | 周期 | PDO 字节 | 主要覆盖 |
| --- | --- | --- | --- | --- |
| `full` | 2/3 | 1 ms | 66 | 三模式、IO、SDO、停止、重新准入、虚拟 I2C |
| `Q1` | 32/32 | 1 ms | 904 | 首轴集成 IO，单 Domain、DC、32 轴生命周期 |
| `Q2` | 16/17 | 500 μs | 459 | 双 Domain、DC、IO、I2C 故障隔离 |
| `Q4` | 16/17 | 1 ms | 459 | 持续八请求、1 KiB SDO 写入及逐字节回读 |
| `Q4-baseline` | 16/17 | 1 ms | 459 | 相同冻结拓扑与运行序列，不提交 SDO 压力 |

故障场景包括 `command_expiry`、`dc_fault`、`wkc_fault`、`link_fault`、
`drive_fault`、`supervisor_restart`。驱动故障通过显式维护请求发出一次
`0x0080`，随后确认零 Controlword；这不会自行清除生命周期锁存或授予运动许可。

`raw` 后端仅在 Linux 编译，显式指定 `--iface`，保持运动 inhibit；它仍要求
网络匹配编译时生成产品。该入口不提供任意设备自动兼容或发布资格。

## 回放和证据

```bash
cargo run --release -p esop-ethercat-linux-port --example r0_r2 -- \
  --backend replay --scenario full --mode csp --cycles 10000 \
  --pcap build/r0-r2/sim/trace.pcap --sidecar build/r0-r2/sim/trace.json \
  --output-dir build/r0-r2/replay
```

回放要求产品、场景、周期和方向 sidecar 一致。每次 TX 必须逐字节匹配，RX 仅
在记录时间可用；不改写地址、index 或 WKC。当前完整回放合同针对本夹具的确定性
捕获；缺少方向/绑定信息的第三方 PCAP 不作为完整场景输入。

每次运行输出：

- `scenario.json`：实际轴/从站/周期、OP、PDO/DC、SDO、停止/恢复、状态与事件摘要。
- `run.json`：完整周期数、直方图、最大值、错误、SDO 容量、流摘要和软件结论。
- `allocations.json`：激活调用区间的分配、释放、重分配；成功场景要求全部为零。
- `trace.pcap`、`trace.json`、`capture-policy.json`：有限容量原始捕获及明确方向。
- `config.json`、`topology.json`：供导入器核验的原始产品和拓扑文件。

捕获容量耗尽时保留前缀并计数，完整流 SHA、周期、错误及最大值仍覆盖整个运行。
前缀不能作为完整回放输入。`--capture-limit` 可配置该固定容量。

```bash
python3 scripts/import-performance-measurements.py \
  --input build/r0-r2/sim/run.json --config build/r0-r2/sim/config.json \
  --topology build/r0-r2/sim/topology.json --expected-commit "$(git rev-parse HEAD)" \
  --output build/r0-r2/sim/performance.json
```

导入器核验实际文件 SHA、commit、拓扑、样本数、直方图和 SDO 容量，并始终保持
软件报告 `qualification.passed=false`。Q4 可通过 `--baseline` 导入相同产品/
拓扑/周期/来源的无负载报告，计算实测 P99 回归；不能提交一个无文件证据的 baseline hash。
基线必须完成软件场景、激活后零分配，且没有提交过 SDO 负载；失败或部分运行不能充当空载参考。
release jitter 使用 port 时间，fast path 使用 host 单调时间，RX round trip 使用
匹配 TX 后实际观察到 RX 的 port 轮询时间；虚拟总线时间不等于宿主执行时长。
RX timing 只统计可明确匹配的每周期最大值。索引在未收到响应前再次使用时，该索引的
RTT 在本次捕获中保持不可用；缺失观测不会填成零延迟。`run.rx_round_trip_samples`
记录实际覆盖周期数；旧版记录没有这个字段时，导入结果明确标为覆盖未知。
字节量和 payload span 是软件观测量，尚非完整 memcpy/DMA 仪器测量；未知目标
arena、stack、DMA、CPU 和代码资源保持未测量。

## 自动检查与长稳入口

```bash
make test-master-software  # 三模式、实际 Q1/Q2/Q4、故障、回放和报告导入
make test-master-soak      # Q4 的 3,600,000 个 1 ms 周期
make r2-release-gate       # 发布资格必须真实 qualified
```

长稳入口执行一小时**虚拟总线时间**，同时记录实际宿主耗时和全部周期的分配/
错误/最大值。它不能替代一小时实物 HIL 或目标实时性能测量。开发 `make ci`
允许合法的未合格基线；当前硬件缺失时 `r2-release-gate` 应拒绝发布。

已保存的[软件长稳证据](../qualification/software_master_soak.json)覆盖 360 万周期、
16 轴/17 从站、35,520 个完成的 SDO；激活后的分配、释放、重分配均为零，
周期通信、DC 和邮箱错误计数为零。该记录绑定源码提交和配置，明确保留抓包
前缀损失、关闭时尚未完成的八个请求及未取得的硬件资格。

## 原始 P0 证据映射

以下为本阶段的软件覆盖，原 PRD 中要求 HIL、物理时间、板级 DMA/cache、真实
缩放/制动或发布审查的部分继续保留原验收条件。

| P0 ID | 软件证据与出口 |
| --- | --- |
| FR-001–004 | Startup/scan/AL/SII、Mapping/PDO/watchdog；独立虚拟 1/8/32 站扫描、生成产品 OP 与精确回读 |
| FR-006 | cfggen 的六个确定性 C/Rust/JSON 产物、容量/hash 与运行时 anti-tamper 测试 |
| FR-007–008 | `wire_qualification` 的 14 个受支持非 NOP 命令 literal bytes、M/WORD 顺序、多数据报与边界 |
| FR-009–011 | core cycle/Domain/RX tests、完整提交、三重预算、无效/旧/重复/迟到帧；捕获回放 |
| FR-012 | slave-copy core/Linux/owner 测试及 `full` 的双 Domain 过程映像复制 |
| FR-014–016 | CoE/mailbox/bank 单测及 `production_sdo` 的真实 SM/MRL、1 KiB、八请求、Emergency、mapped status 和失败隔离 |
| FR-018–019 | 现有 DC discovery/programming/SYNC/window tests、运行示例的 DC 丢锁与运动门槛 |
| FR-020–022 | CiA 402 profile tests、三个生成模式、实际 PDS 使能、停止、维护复位和重新准入 |
| FR-024–025 | `esop-device` 的统一状态模型、drive/IO profile tests、虚拟 I2C 的 Pending/NACK/timeout 与显式恢复 |
| FR-026–028 | ProcBuf ABI/hash/boot/并发/过期 tests、SI/raw 命令与反馈、生成映射和 owner 发布 |
| FR-034–036 | 固定诊断/terminal/Emergency/monitor events、年龄快照与六类故障；溢出可计数 |
| FR-038 | artifact-bound report import、完整周期/max/error 与强制未合格的软件来源 |
| FR-039–043 | guard/model/owner tests 和场景中的未知/失效门、停止确认、锁存、显式 clear 与新 epoch permit |
| FR-045–046 | ProcBuf 状态/事件一致性、维护 inhibit、owner-bound Fault Reset 和配置/恢复门槛 |
| NFR-001、017 | 真实 thread-local allocator、固定容量观测/请求、每周期完整测量与 guard tests |
| NFR-002–003、005 | 实际 32/16/16 轴 Q1/Q2/Q4 软件场景；真实目标 jitter/WCET/P99 门槛仍由 HIL 验证 |
| NFR-006 | 生成资源/SDO 存储预算、固定容量约束；目标 map/stack/arena 实测仍待提交 |
| NFR-007 | 独立线格式、故障/恶帧、完整输入提交、同配置捕获回放与错误保持 |
| NFR-008–009、011 | 现有 DMA/所有权/ProcBuf/32 位保护合同与 no_std gate；MCU 端本阶段延后 |
| NFR-010 | 显式 rescan/reconfigure/state、真实 token handoff、故障清除不自动恢复运动 |
| NFR-012 | Linux raw 开发入口及软件夹具；STM32/HPMicro 和物理端口资格延后 |
| NFR-015–016 | 现有安全/独立实现/来源许可边界继续适用，软件场景不授予认证或硬件支持标签 |

软件验收记录位于 `build/r0-r2/qualification/qualification.json`；CI 同时上传
逐场景证据。能力声明以 `capability_manifest.json` 和各项限制为准。
