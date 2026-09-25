# ESOP Zenoh 路由边界

- 文档版本：1.0
- 日期：2026-09-25
- 状态：固定 key namespace、方向策略、可选 Zenoh Session、与 IPC 共享的 ProcBuf v4 状态/事件投影和 MotionCommand 字段解码、逐轴停止证据与原始周期质量投影、eBPF RuntimeIncident 无损投影、类型化查询边界、host QoS、稳定 publish 与 command/query callback 观测 marker、生产安全配置准入及 loopback router 验证已实现；真实设备质量采集、远程 ACL、命令 target 写入、目标内核 uprobe/开销和生产认证部署待完成
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
- `publish_agent_incident`：在 host 监督域校验 `esop-ebpf-agent::RuntimeIncident` 的非零 identity、time/cycle window、1-8 条 evidence、boot/epoch 和 evidence 所属范围，再生成确定性全局 incident ID 并投影完整 64-bit/provenance 字段。旧 `RuntimeEvidence.value` 只保存饱和的 32-bit 摘要；投影失败与 route/schema/Zenoh transport 错误分离，且不会启动 transport publish marker；
- `subscribe_commands`：在固定 `cmd` key 上注册后台 subscriber；
- `admit_authenticated_command`：要求可信监督服务先将认证主体映射为固定 `source_id`，再比较 transport identity 与 payload identity；不匹配时不会进入实时准入；
- `serve_queries`：在固定 `query` key 上注册底层后台 queryable；
- `serve_typed_queries`：绑定当前 boot ID，校验 `QueryRequest` 的 payload（最大 4096 bytes）、schema 版本、robot ID 和 `limit`（1-32），将请求交给监督域的快照提供者；应答校验 `QueryReply` 及嵌套记录的版本、robot/boot、incident/证据稳定 ID、共享 agent epoch、窗口/周期范围、证据数量和编码后的大小，不符合条件时返回稳定的 Zenoh error reply，不发送数据回复。`after_sequence` 对返回的 `RobotState.sequence` 实施严格递增检查；incident 的查询/分页语义由提供者定义，不应误用 `cycle_sequence` 作为状态序号。请求与回复失败计数可通过 `TransportHealth` 读取。
- `TransportHealth`：记录连接状态、发布失败数和 handler 注册数。
- `PublishQos`：state 使用可丢弃的 data 队列，event/diagnostic 使用可丢弃的高优先级队列；不会因 Zenoh 背压阻塞监督域任务。
- publish 观测 ABI：用稳定的 `esop_zenoh_gateway_publish_begin_v1(request_id, route_kind)` / `esop_zenoh_gateway_publish_end_v1(request_id, route_kind, outcome)` 标记完整异步 publish 生命周期；request ID 为进程内非零原子序号，outcome 固定为 success、transport failure 或 cancellation。
- callback 观测 ABI：用独立稳定的 `esop_zenoh_gateway_callback_begin_v1(request_id, route_kind)` / `esop_zenoh_gateway_callback_end_v1(request_id, route_kind, outcome)` 包围 command subscription 与 raw/typed query callback 调用；publish 和 callback 共用同一非零 request ID 序列，正常返回固定为 completed，Rust unwind 通过 guard `Drop` 固定为 abandoned。类型化 query 的 decode、provider、encode 和同步 reply wait 均位于同一 query callback 观测窗口内。
- `TransportSecurityPolicy` / `open_secure`：在建立 Session 前检查传输协议、TLS 根证书、名称校验、mTLS 客户端证书/私钥和公钥或用户名密码认证材料；生产调用方应使用 `TransportSecurityPolicy::production()`，开发/HIL 可显式使用 `open` 或 `development()`。
- `decode_command` / `admit_command`：复用 `esop-ipc/payloads` 的 `MotionCommand` 字段解码，校验 schema 和 robot ID，并转交 `CommandIngress` 执行来源、权限、TTL、epoch、序号、轴掩码、限流和审计。Zenoh 仍负责 namespace 与认证主体映射；共享解码器不授予 permit。
- `ProcBufProjector`：Zenoh 兼容 wrapper 委托给 `esop-ipc/payloads` 的监督域单读者。共享实现校验 ABI/layout、数值 robot ID 和 boot ID 后读取完整状态页与事件环，转换为外部 `RobotState` / `DiagnosticEvent`；原样投影 required/valid/qualified/ready 门槛位图、permit epoch/expiry、转换周期、恢复计数和 permit 审计序号，并在原始质量事实完整且与 State 序号相同时生成 `QualitySummary`；拒绝状态序号回退、无效生命周期值、非法质量位图、非有限关节值和超 4096-byte 状态。数值 robot ID 与外部文本 ID 的配对须由部署配置提供，不从字符串猜测哈希。

marker 是无阻塞、无字符串解析的可选观测边界；没有附加 uprobe 时只执行固定参数的空 marker，不改变 publish 或 callback 结果。Rust `async fn` 的普通 entry/return 只覆盖 future 构造，不能表示 await 生命周期，因此 publish guard 在 Session put 前 begin，并在健康刷新、传输失败处理或 future drop 后恰好 end 一次。callback guard 则紧贴 gateway 所有的调用边界，在调用用户 callback 前 begin，并在正常返回或 Rust unwind 时恰好 end 一次；进程 abort 不伪造 terminal marker，遗留状态由固定容量 LRU 约束。eBPF runtime 分别对 publish 和 callback 的 begin/end 符号实行原子成对附加，缺失可选符号时保留内核 tracepoint 基线；是否把任一探针对设为 required 由监督域显式决定。该 ABI 不提供运动许可、投递确认或功能安全保证。

示例编译检查：

```bash
cargo check -p esop-zenoh-gateway --features zenoh
```

现场 router smoke test（需要已安装 `zenohd 1.10.1`）：

```bash
cargo install zenohd --version 1.10.1 --locked
make test-zenoh
```

特权托管 Linux 的 marker-to-incident 资格入口：

```bash
make test-ebpf-gateway-runtime
```

该入口以普通用户编译 BPF 与 Rust，仅提升最终夹具执行；夹具关闭无关 tracepoint，
要求自身 publish/callback 四个精确 marker 符号，以同一 WKC-risk cycle 分别注入
25 ms 的 Diagnostic/success publish 与 Command/completed callback 延迟，并要求两条
证据合并为一个 `GatewayStall` incident、统计与附着位自洽且零 mismatch/loss。成功后
生成并严格校验 `build/ebpf_gateway_qualification.json`。它验证共享 marker、CO-RE
verifier/load、真实 uprobe、ringbuf 和相关器链路，不验证 live Zenoh Session、router/
transport queue、IPC、序列化、permit、reconnect、开销/WCET 或生产实时环境。

测试脚本只监听动态分配的 loopback TCP 端口，退出时自动关闭 router；它不是实时周期依赖，也不代表生产环境已经完成认证、远程 ACL、证书生命周期或重连配置。

callback 运行在 Zenoh host runtime：命令 callback 应只把数据投递到有界命令队列，query callback 可完成查询应答，但两者都不能直接操作 EtherCAT 周期或绕过 `esop-command-gateway`。会话关闭或传输失败会将状态标为 `Disconnected` 或 `Degraded`；重连不会自动恢复运动许可。

类型化 query provider 必须只读取监督域快照，并自行限制执行时间和并发；`limit` 限制结果记录数及响应字节数，不是远程身份认证或请求速率限制。生产环境仍须在传输层配置调用方身份、ACL 和限流；robot boot 变化时应关闭旧 gateway 并重新绑定 queryable，不得继续服务旧 boot 的快照。

`ProcBufProjector` 的输出是独立的有所有权快照。一个监督域 reader 负责更新缓存，发布者和 query provider 使用同一份已校验的快照，不得各自竞争 ProcBuf 双页的单读者所有权。`RobotState.joints`/`io` 中的 index 从 0 开始，直接复制已经由 profile 换算的 SI 值；事件使用独立的 SPSC 环，`RobotState.events` 不隐式混入事件。实时端可启用 `esop-lifecycle-guard/procbuf`，用 `lifecycle_to_procbuf(snapshot, transition_time_ns)` 无分配地产生同一份生命周期摘要；时间参数必须来自对应转换记录，不能把当前周期时间冒充历史转换时间。`cyclic_quality_to_procbuf(&mut state, observations)` 从待发布的 State 页获取序号，把周期所有者采集的 11 项原始布尔事实写入固定双位图：`known_mask` 表示有观测，`good_mask` 表示实际为真。它与 MLG 去抖后的门槛位图不同；其中 `configuration_ready` 对应现有 `CyclicQuality.coe_ready`，不是通用配置认证。质量序号不等于 State 序号或已知位图不完整时，`RobotState.quality` 为 `None`；序号匹配但位图有非法位则拒绝整个投影，已完整观测但所有位均为 false 时仍发布真实的全 false 摘要。`first_fault_code` 沿用同周期 MLG 的 `first_blocking_code`，不伪造原始诊断代码。`RobotState` 是观测数据，不是运动许可或功能安全判断。

`motion_permit_current` 仅表示 permit 本身未过期，不表示当前可以驱动：命令门槛失效时，状态可能已进入 `Stopping`，而 permit 仍在有效期。执行侧始终以 MLG 状态和门槛决策约束 CiA 402，不以该布尔字段单独判定运动授权。

ProcBuf 固定布局已升级到 ABI v4；v1/v2/v3 reader/writer 不能复用 v4 区域，attach 时必须核对 version、layout hash、容量、robot 和 boot ID。升级需停止旧实时端与监督进程并重新创建区域，再启动相同版本的双方；旧 header 在单元测试中明确被拒绝。Protobuf 仍为 v1 外部契约，新增 `LifecycleSummary.axis_stops` 只携带本周期请求/发出动作和合格的驱动反馈证明位，不表示真实执行的停止动作；旧 Protobuf 读者丢弃新增字段，不能充当透明中继。

补充验证：测试进程为每个场景动态申请 loopback 临时端口并独占 zenohd，覆盖 state/event、从固定 agent incident 经生产投影到 router 解码的完整 v1 payload、router 重启后的 health recovery、旧命令 TTL/代际拒绝，以及恢复必须经过新 permit 和显式 rearm。ZenohGateway::refresh_health 使用 session 的 router/peer 连接快照；它属于 host supervisor 观察，不是 motion permit 或应用层投递确认。[Zenoh SessionInfo API](https://docs.rs/zenoh/1.10.1/zenoh/session/struct.SessionInfo.html)
