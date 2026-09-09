# ESOP Zenoh 路由边界

- 文档版本：1.0
- 日期：2026-09-09
- 状态：固定 key namespace、方向策略和 payload contract 已实现；Zenoh transport 待集成
- 上游需求：PRD FR-031、FR-030、FR-045、FR-051

## 1. Key namespace

所有路由使用固定形式：

```text
esop/<fleet>/<robot_id>/{state,event,diagnostic,cmd,query}
```

`fleet` 和 `robot_id` 由 `KeySpace` 在创建时验证并复制到固定数组。key 由调用方提供的固定 buffer 生成，不分配内存。不同 robot 的 key 不会被另一 `KeySpace` 接受，避免跨机器人订阅或命令串线。

## 2. 路由方向与消息

| 路由 | 方向 | `esop.v1` payload |
| --- | --- | --- |
| `state` | 监督域发布 | `RobotState` |
| `event` | 监督域发布 | `DiagnosticEvent` |
| `diagnostic` | 监督域发布 | `RuntimeIncident` |
| `cmd` | 监督域接收 | `MotionCommand` |
| `query` | 双向 | `QueryRequest` / `QueryReply` |

路由层只校验 key、方向和 payload 类型；它不执行 Protobuf 编解码，也不承担认证。实际 payload 必须先经过生成的 v1 binding，再由命令准入层执行 source/authority/TTL/sequence/axis policy，最后才转换为 `MotionPermit`。

## 3. 断连与实时隔离

Zenoh session、router、发现、重连、QoS 和 transport security 均属于 Linux 监督域。它们不可进入 EtherCAT 周期、MLG 决策或 ProcBuf 的必要执行链。断连时，实时域继续按已有命令/permit 的 deadline 和 MLG 策略运行；过期后停止，不因重连自动恢复运动。

当前测试覆盖 robot 隔离、发布/订阅方向、payload contract、非法 key、固定 key buffer 和 payload 边界。Zenoh session/router 真实连通、ACL、断连重连、schema compatibility 和 QoS 测试仍是后续阶段。
