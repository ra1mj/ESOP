# ESOP Protobuf v1 契约

- 文档版本：1.0
- 日期：2026-09-25
- 状态：schema 源、结构校验、Rust 生成绑定、v1 版本准入、冻结基线 descriptor 门禁、新旧 reader/writer 双向测试、RuntimeIncident additive 无损字段和 loopback transport round-trip 已实现；跨语言与生产滚动升级仍待验证
- 上游需求：PRD FR-030、FR-031、FR-045、FR-049、FR-051

## 1. 边界

`proto/esop/v1/esop.proto` 是 Linux 监督域、Zenoh gateway、ROS 2 bridge 和运维工具共享的外部数据契约。它不进入 EtherCAT PDO 周期、MLG 决策或 ProcBuf 的链接依赖图。

当前契约覆盖：

- `RobotState`、`JointState`、`IoState` 和 `QualitySummary`
- `LifecycleSummary`（包括可选逐轴 `AxisStopEvidence`）、`DiagnosticEvent` 和审计关联字段
- `MotionCommand`、`CommandReply` 和外部命令来源/TTL/序号/策略版本
- `RuntimeIncident`、`RuntimeEvidence`、查询请求与查询响应；incident 追加字段保留 boot/agent epoch、配置证据窗口、完整 cycle 范围、组件标识、置信度和 64-bit 观测值，evidence 追加字段保留稳定 ID、epoch、domain、transition、IRQ/ifindex、duration/count/detail 和完整 64-bit 值

## 2. 演进规则

1. 包名固定为 `esop.v1`；破坏性变更必须创建新的版本包，不覆盖 v1。
2. 已发布字段号和字段名不得复用；删除字段必须同时保留 `reserved` 字段号和名称。
3. 每个 enum 的零值必须是带 `_UNSPECIFIED` 后缀的显式未知值；未知枚举不能被解释为正常状态。
4. 状态、事件、命令和 incident 都携带可关联的 robot/boot/sequence/time 字段，网关不得从 key 名称推断消息语义。
5. 所有顶层消息携带 `schema_version`；当前唯一接受值为 `1`，缺失或未知版本必须在网关边界拒绝。
6. 当前 Prost reader 会忽略未知字段，重新编码时不会保留它们；不能作为透明中继，也不能绕过 `schema_version` 准入。破坏性变更必须创建新的版本包。
7. 生成代码、Protobuf 编解码和 Zenoh payload 只在监督域使用；实时节点只接收已经转换并校验过的固定命令/permit。
8. `RuntimeEvidence.value` 保留为旧 reader 的饱和 32-bit 摘要；新增 `observed_value` 是完整 64-bit 权威值。incident ID 由 boot ID、agent epoch 和 agent-local ID 确定性组成，不能只使用重启后会复用的本地序号。

## 3. CI 校验

`make proto-schema` 检查 proto3/package、顶层消息的 `schema_version`、字段号和名称唯一性、reserved 不复用及 enum 零值。`crates/esop-proto/` 使用 vendored `protoc` 生成 Rust bindings；workspace 测试另外执行第 4 节的 descriptor 门禁和新旧 reader/writer 矩阵。`esop-zenoh-gateway` 的命令入口校验 schema version 和 robot ID 后再转交 `CommandIngress`；host-only incident adapter 在编码前校验 agent identity、窗口、证据数量、boot/epoch 和证据所属范围。类型化查询入口校验请求的 schema、robot、boot 和大小，并对 incident 及其 evidence 重做相同的 Protobuf 合同校验。`make test-zenoh` 验证类型化发布、真实 agent incident 投影、命令路由、查询往返/拒绝及 router 重启后的旧命令拒绝。生产认证和部署仍是后续验收项。

## 4. Frozen v1 Compatibility Gate

PRD FR-030 and the R0/R3 schema gates are covered by
`crates/esop-proto/tests/schema_compatibility.rs` and `reader_writer.rs`.
Both run under `cargo test -p esop-proto` and the workspace `make ci` gate.

The fixture `tests/fixtures/v1_784bf73.proto` is frozen from commit `784bf73`,
after explicit schema-version admission was introduced. Do not regenerate this
fixture when editing the current schema. The build compiles the two schemas
independently; baseline bindings are included only by integration tests.

| Writer / Reader | Automated evidence |
| --- | --- |
| Frozen v1 / Current v1 | Seven top-level messages, including representative nested state, command targets and incident evidence |
| Current v1 / Frozen v1 | The same non-default values survive decoding with the frozen generated bindings |
| Additive test writer / Both v1 readers | Unknown fields are ignored; decode/re-encode drops them |
| Current v1 axis stop evidence / Frozen v1 reader | Old reader preserves legacy summary but ignores new per-axis evidence; re-encoding loses the evidence |
| Current v1 runtime incident / Frozen v1 reader | Old reader preserves legacy incident/evidence fields and saturated 32-bit value, ignores additive provenance/full-width fields, and drops them on re-encode |
| Unknown enum writer / Both v1 readers | Numeric value remains unknown and typed conversion fails |

The descriptor gate rejects package/syntax changes, removed messages, field
renumbering, renaming, type/cardinality/presence/oneof changes, changed enum
values and removed reservations. Field deletion requires both the name and
number to be reserved. Existing enum values remain stable under this gate.
Additive fields are allowed; semantic compatibility still requires review.
Mutation tests verify that the gate rejects deliberate incompatible changes.

Pre-`784bf73` writers without `schema_version` are not admitted by the current
gateway. This is not a claim of rolling-upgrade compatibility with those writers.
Cross-language readers, new major schema packages, transparent relays,
application semantic changes and production rolling upgrades remain unqualified.
