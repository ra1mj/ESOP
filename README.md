# ESOP

**EtherCAT Simple Operating System**：以 Rust `no_std` 核心为基础的 EtherCAT
实时控制框架，主机侧提供产品配置生成、只读诊断和可选 eBPF 观测。

目前是 **0.1.0 开发框架**，还不是与 SOEM 2.0 功能齐全对齐的 V1：FoE、SoE、EoE、
双网卡冗余、ENI 和完整主站服务/维护命令仍待实现，实物 HIL/WCET 也未完成。
具体矩阵、CLI 验证和体积测量见 [V1 审计](docs/v1-readiness-audit.md)。

## 快速开始

Linux 开发环境需要 Rust **1.88+**、Python 3 和 GCC。普通构建和测试不需要 root。
体积测量另需 binutils（`size`、`nm`、`strip`）。

```sh
rustup target add aarch64-unknown-none
make core-check             # 仅检查 no_std 核心与产品配置，不构建主机传输
make core-example           # 编码/解析一个 EtherCAT 帧，不访问网卡
make host-build             # 构建 esop 与 esop-cfggen 两个主机工具
target/release/esop --help
target/release/esop-cfggen --help
make cfggen-runtime-example # 从示例产品生成六个产物并核对黄金文件
```

`make core-example` 是带宿主标准库的最小 wire 层演示，不代表完整主站或目标固件体积。
目标部署还需要产品拓扑、平台驱动、运行时集成和相应资格证据。

## 诊断命令

下面的查询需要已有应用通过 `ZenohGateway::serve_typed_queries` 提供状态。
仓库尚未交付可直接启动的主站/监督 daemon；仅启动 Zenoh router 不会产生设备状态。

```sh
target/release/esop --robot robot_01 --config /path/to/zenoh.json5 status
```

| 命令 | 用途 |
| --- | --- |
| `status` | 汇总运行状态、质量、Domain、DC、生命周期和观测健康 |
| `domain list` | 查看每个 Domain 的 WKC、完整性和输入年龄 |
| `dc` | 查看锁定、偏差和质量门 |
| `lifecycle` | 查看状态、permit 和阻塞原因 |
| `incident list` | 查看经过校验的运行时 incident |
| `doctor` | 执行健康检查；healthy 返回 0，degraded 返回 3 |
| `watch [上述任一命令]` | 持续查询；支持 `--interval-ms` 和 `--iterations` |

命令保持只读，不启用运动、不恢复从站、不加载 BPF。
参数、增量语义与错误码见 [CLI 合同](docs/esop-runtime-diagnostics-cli.md)。
完整 SOEM 维护命令的缺口见 [V1 审计](docs/v1-readiness-audit.md)。

## 验证与体积

```sh
make ci                     # 格式、测试、Clippy、构建、裸机核心和证据校验
cargo install zenohd --version 1.10.1 --locked
make test-zenoh              # 真实 router、typed provider 和 CLI 进程测试
make footprint              # 生成 build/footprint_report.json
```

`make footprint` 分别测量主机二进制、wire 示例、依赖数和一次受控 CLI 启动等待的
RSS/线程采样。它不推算嵌入式 RAM、flash、WCET 或完整发行包大小。
特权 eBPF 资格入口及限制见 [框架详解](docs/framework-overview.md)。

## 设计与证据

- [EtherCAT 主站需求](docs/ethercat-master-requirements.md)
- [机器人软件规划](docs/robotics-esop-software-plan.md) · [实时架构决策](docs/esop-performance-architecture-decision.md)
- [ETG 与 CiA 402 决策](docs/esop-etg-cia402-master-requirements.md)
- [产品配置生成器](docs/esop-product-configuration.md)
- [eBPF 观测设计](docs/esop-ebpf-runtime-observability.md)
- [能力声明](capability_manifest.json) · [资格限制](qualification/KNOWN_LIMITATIONS.md)
- [开发历史及保留证据](docs/development-history.md)

项目按根目录 [MIT LICENSE](LICENSE) 授权；第三方依赖保留各自许可证。
