# ESOP Zenoh 路由边界

- 文档版本：1.0
- 日期：2026-09-20
- 状态：固定 key namespace、方向策略、可选 Zenoh Session、ProcBuf 状态/事件投影、v1 命令和类型化查询边界、host QoS、生产安全配置准入及 loopback router 验证已实现；完整状态语义、远程 ACL 和生产认证部署待完成
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

当前单元测试覆盖 robot 隔离、发布/订阅方向、payload contract、非法 key、固定 key buffer 和 payload 边界；`make test-zenoh` 会为每个测试动态申请 loopback 端口并启动本地 `zenohd 1.10.1`，验证 state/event/incident publish、command subscribe/准入审计、类型化 query reply 和拒绝响应、router 重启恢复、close 后的断连状态和 state publication 的实际 QoS。安全配置单元测试验证生产策略拒绝默认/明文配置，并要求 TLS、名称校验、mTLS 客户端材料和认证材料。远程 ACL、证书颁发/轮换及生产拓扑测试仍是后续阶段；Protobuf 的冻结 v1 兼容门禁见 `esop-protobuf-v1.md`。

## 4. Real Zenoh adapter

`esop-zenoh-gateway` 默认保持 `no_std`。Linux 监督域启用 `zenoh` feature
后，`runtime::ZenohGateway::open` 创建真实 `zenoh::Session`，并提供：

- `publish`：先复用 `KeySpace` 的方向、payload contract 和 4096-byte 上限校验，再执行 Session put；
- `publish_state` / `publish_event` / `publish_incident`：使用生成的 `esop.v1` 类型编码后发布，状态快照额外校验 `robot_id` 与 namespace 一致；
- `subscribe_commands`：在固定 `cmd` key 上注册后台 subscriber；
- `admit_authenticated_command`：要求可信监督服务先将认证主体映射为固定 `source_id`，再比较 transport identity 与 payload identity；不匹配时不会进入实时准入；
- `serve_queries`：在固定 `query` key 上注册底层后台 queryable；
- `serve_typed_queries`：绑定当前 boot ID，校验 `QueryRequest` 的 payload（最大 4096 bytes）、schema 版本、robot ID 和 `limit`（1-32），将请求交给监督域的快照提供者；应答校验 `QueryReply` 及嵌套记录的版本、robot/boot（含 evidence）、记录总量和编码后的大小，不符合条件时返回稳定的 Zenoh error reply，不发送数据回复。`after_sequence` 对返回的 `RobotState.sequence` 实施严格递增检查；incident 的查询/分页语义由提供者定义，不应误用 `cycle_sequence` 作为状态序号。请求与回复失败计数可通过 `TransportHealth` 读取。
- `TransportHealth`：记录连接状态、发布失败数和 handler 注册数。
- `PublishQos`：state 使用可丢弃的 data 队列，event/diagnostic 使用可丢弃的高优先级队列；不会因 Zenoh 背压阻塞监督域任务。
- `TransportSecurityPolicy` / `open_secure`：在建立 Session 前检查传输协议、TLS 根证书、名称校验、mTLS 客户端证书/私钥和公钥或用户名密码认证材料；生产调用方应使用 `TransportSecurityPolicy::production()`，开发/HIL 可显式使用 `open` 或 `development()`。
- `decode_command` / `admit_command`：解码 `esop.v1.MotionCommand`，校验 robot ID，并转交 `CommandIngress` 执行来源、权限、TTL、epoch、序号、轴掩码、限流和审计。
- `ProcBufProjector`：监督域单读者在校验 ABI/layout、数值 robot ID 和 boot ID 后读取完整状态页与事件环，转换为外部 `RobotState` / `DiagnosticEvent`；拒绝状态序号回退、无效生命周期值、非有限关节值和超 4096-byte 状态。数值 robot ID 与外部文本 ID 的配对须由部署配置提供，不从字符串猜测哈希。

示例编译检查：

```bash
cargo check -p esop-zenoh-gateway --features zenoh
```

现场 router smoke test（需要已安装 `zenohd 1.10.1`）：

```bash
cargo install zenohd --version 1.10.1 --locked
make test-zenoh
```

测试脚本只监听动态分配的 loopback TCP 端口，退出时自动关闭 router；它不是实时周期依赖，也不代表生产环境已经完成认证、远程 ACL、证书生命周期或重连配置。

callback 运行在 Zenoh host runtime：命令 callback 应只把数据投递到有界命令队列，query callback 可完成查询应答，但两者都不能直接操作 EtherCAT 周期或绕过 `esop-command-gateway`。会话关闭或传输失败会将状态标为 `Disconnected` 或 `Degraded`；重连不会自动恢复运动许可。

类型化 query provider 必须只读取监督域快照，并自行限制执行时间和并发；`limit` 限制结果记录数及响应字节数，不是远程身份认证或请求速率限制。生产环境仍须在传输层配置调用方身份、ACL 和限流；robot boot 变化时应关闭旧 gateway 并重新绑定 queryable，不得继续服务旧 boot 的快照。

`ProcBufProjector` 的输出是独立的有所有权快照。一个监督域 reader 负责更新缓存，发布者和 query provider 使用同一份已校验的快照，不得各自竞争 ProcBuf 双页的单读者所有权。`RobotState.joints`/`io` 中的 index 从 0 开始，直接复制已经由 profile 换算的 SI 值；事件使用独立的 SPSC 环，`RobotState.events` 不隐式混入事件。当前 ProcBuf lifecycle 只提供状态、停止动作、ready gate mask、故障码、motion permit 布尔值和转换/恢复序号，缺少 required/valid/qualified masks、permit epoch/expiry 等字段；投影会保留这些缺口为默认值，不能据此重建完整 MLG 资格。`QualitySummary` 暂不发布（`None`），因为缺少其全部门槛事实；`RobotState` 是观测数据，不是运动许可或功能安全判断。

补充验证：测试进程为每个场景动态申请 loopback 临时端口并独占 zenohd，覆盖 state/event/incident 的 v1 payload、router 重启后的 health recovery、旧命令 TTL/代际拒绝，以及恢复必须经过新 permit 和显式 rearm。ZenohGateway::refresh_health 使用 session 的 router/peer 连接快照；它属于 host supervisor 观察，不是 motion permit 或应用层投递确认。[Zenoh SessionInfo API](https://docs.rs/zenoh/1.10.1/zenoh/session/struct.SessionInfo.html)
