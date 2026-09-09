# ESOP Zenoh 路由边界

- 文档版本：1.0
- 日期：2026-09-09
- 状态：固定 key namespace、方向策略、payload contract、可选 Zenoh Session 适配器和 v1 命令准入桥接已实现；现场 router 验证待集成
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

## 4. Real Zenoh adapter

`esop-zenoh-gateway` 默认保持 `no_std`。Linux 监督域启用 `zenoh` feature
后，`runtime::ZenohGateway::open` 创建真实 `zenoh::Session`，并提供：

- `publish`：先复用 `KeySpace` 的方向、payload contract 和 4096-byte 上限校验，再执行 Session put；
- `subscribe_commands`：在固定 `cmd` key 上注册后台 subscriber；
- `serve_queries`：在固定 `query` key 上注册后台 queryable；
- `TransportHealth`：记录连接状态、发布失败数和 handler 注册数。
- `decode_command` / `admit_command`：解码 `esop.v1.MotionCommand`，校验 robot ID，并转交 `CommandIngress` 执行来源、权限、TTL、epoch、序号、轴掩码、限流和审计。

示例编译检查：

```bash
cargo check -p esop-zenoh-gateway --features zenoh
```

callback 运行在 Zenoh host runtime：命令 callback 应只把数据投递到有界命令队列，query callback 可完成查询应答，但两者都不能直接操作 EtherCAT 周期或绕过 `esop-command-gateway`。会话关闭或传输失败会将状态标为 `Disconnected` 或 `Degraded`；重连不会自动恢复运动许可。
