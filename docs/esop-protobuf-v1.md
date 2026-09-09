# ESOP Protobuf v1 契约

- 文档版本：1.0
- 日期：2026-09-09
- 状态：schema 源、结构校验、Rust 生成绑定、Zenoh 命令解码适配和 loopback transport round-trip 已实现；跨版本运行时矩阵待集成
- 上游需求：PRD FR-030、FR-031、FR-045、FR-049、FR-051

## 1. 边界

`proto/esop/v1/esop.proto` 是 Linux 监督域、Zenoh gateway、ROS 2 bridge 和运维工具共享的外部数据契约。它不进入 EtherCAT PDO 周期、MLG 决策或 ProcBuf 的链接依赖图。

当前契约覆盖：

- `RobotState`、`JointState`、`IoState` 和 `QualitySummary`
- `LifecycleSummary`、`DiagnosticEvent` 和审计关联字段
- `MotionCommand`、`CommandReply` 和外部命令来源/TTL/序号/策略版本
- `RuntimeIncident`、`RuntimeEvidence`、查询请求与查询响应

## 2. 演进规则

1. 包名固定为 `esop.v1`；破坏性变更必须创建新的版本包，不覆盖 v1。
2. 已发布字段号和字段名不得复用；删除字段必须同时保留 `reserved` 字段号和名称。
3. 每个 enum 的零值必须是带 `_UNSPECIFIED` 后缀的显式未知值；未知枚举不能被解释为正常状态。
4. 状态、事件、命令和 incident 都携带可关联的 robot/boot/sequence/time 字段，网关不得从 key 名称推断消息语义。
5. 生成代码、Protobuf 编解码和 Zenoh payload 只在监督域使用；实时节点只接收已经转换并校验过的固定命令/permit。

## 3. CI 校验

`make proto-schema` 会在没有安装 protobuf runtime 的开发机上检查 `proto/esop/v1/*.proto` 的 proto3/package 声明、消息字段号和字段名唯一性、reserved 不复用及 enum 未定义零值。`crates/esop-proto/` 使用 vendored `protoc` 生成 Rust bindings，并通过 encode/decode 单测验证 v1 消息；`esop-zenoh-gateway` 会校验 robot ID 后再把 `MotionCommand` 转交固定容量 `CommandIngress`。`make test-zenoh` 进一步验证 v1 state/event/incident payload、command payload 经真实 loopback router 到达 subscriber，并验证 query reply 与 router 重启后的旧命令拒绝。旧/新 reader-writer 兼容组合、认证身份和生产部署仍是后续验收项。

Live router 测试还验证 v1 的 state、event、incident payload，以及 router 重启后旧命令的 TTL/代际拒绝；认证身份绑定与生产部署仍由监督域认证服务负责。
