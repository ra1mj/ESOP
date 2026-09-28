# ESOP 运行时诊断命令

- 文档版本：1.0
- 日期：2026-09-28
- 状态：初始只读命令集、类型化 Zenoh 查询、零 boot 发现、Domain/DC/生命周期/eBPF 渲染、doctor 和 bounded watch 已实现
- 上游需求：PRD FR-030、FR-031、FR-035、FR-045、FR-047、FR-051

## 1. 目标与边界

`crates/esop-cli` 提供类似 ROS 2 introspection 的稳定命令入口，但数据源仍是 ESOP
自身的 ProcBuf、Protobuf 和 Zenoh 合同。命令只在 Linux 监督域运行，不进入 EtherCAT
周期，不直接读取过程映像，不加载/卸载 BPF，不写 MotionCommand，不改变 AL/DC/生命周期，
也不确认或恢复故障。

## 2. 命令

```text
esop --robot <id> status
esop --robot <id> domain list
esop --robot <id> dc
esop --robot <id> lifecycle
esop --robot <id> incident list
esop --robot <id> doctor
esop --robot <id> watch [status|dc|lifecycle|doctor|domain list|incident list]
```

公共参数包含 `--fleet`、`--boot-id`、`--limit`、`--config` 和 `--timeout-ms`。
`watch` 另接受 50-60000 ms 的 `--interval-ms`，以及测试/资格场景使用的有限
`--iterations`。默认 boot 为 0，表示只读发现当前 provider boot；首次成功应答后 watch
固定该非零 boot，并将最新 State sequence 用作下一请求的 `after_sequence`。

## 3. 状态语义

- `status` 汇总 robot/boot/sequence、link/AL、fault、命令年龄、deadline 累计、DC、
  Domain、生命周期和 eBPF observation；缺字段显示 `unavailable`，不会按健康处理。
- `domain list` 按 ProcBuf 固定顺序显示 exact expected/actual WKC、valid、complete、连续
  失配数、最近有效周期和输入 age。乱序 Domain 是合同错误。
- `dc` 同时显示 raw lock/offset 和 Quality gate，二者任一缺失或 false 都不是健康。
- `lifecycle` 显示状态、permit、gate masks、首阻塞码、锁存故障、转换和恢复计数。
- `incident list` 显示经合同验证的 eBPF RuntimeIncident，不把 suggested action 当作自动命令。
- `doctor` 对现有事实执行 fail-closed 检查；运行状态/质量/生命周期/eBPF 证据缺失、
  link/DC/Domain/WKC/MLG gate 异常、eBPF loss/degraded/failed 或 error/critical incident
  都返回 degraded。累计 deadline 计数只展示，当前预算结论来自 Quality gate。

eBPF health 数字沿用 ProcBuf 的 `ObservationState`：0 healthy、1 degraded、2 failed；
`agent_epoch=0` 即使 health 字段为 0 仍表示未提供有效 agent 证据。

## 4. 退出与错误

| 退出码 | 含义 |
| --- | --- |
| 0 | 查询和渲染成功；doctor 为 healthy |
| 2 | 参数或命令错误 |
| 3 | doctor 完成但结果 degraded |
| 4 | fleet/robot/Zenoh 配置或 session 创建失败 |
| 5 | 调用方 wall-clock timeout |
| 6 | Zenoh 发送/接收、远端拒绝、Protobuf decode 或查询合同失败 |
| 7 | 返回状态存在未知 enum、乱序 Domain 或非法 observation health |

## 5. 证据与限制

纯逻辑测试覆盖所有命令、help、缺参、未知命令、limit/interval/iterations 边界以及
healthy/degraded 渲染。`make test-zenoh` 使用真实 loopback `zenohd 1.10.1`，验证零 boot
查询同时返回 ProcBuf 投影的运行状态和一条 eBPF incident。该证据不证明远程 ACL、证书
生命周期、生产 provider retention、真实网卡/从站、DC 纳秒精度、目标 WCET、长期压力、
ROS 2 discovery、实物 HIL 或功能安全。
