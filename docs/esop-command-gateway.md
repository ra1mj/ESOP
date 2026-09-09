# ESOP 外部命令准入层

- 文档版本：1.0
- 日期：2026-09-09
- 状态：固定策略与 permit 转换已实现；Zenoh/Protobuf 传输待集成
- 上游需求：PRD FR-031、FR-044、FR-045、NFR-014

## 1. 边界

外部网络、ROS 2、Zenoh 或维护接口不能直接写入实时 Command page，也不能直接修改 CiA 402 controlword。`esop-command-gateway` 位于监督域，负责在进入实时边界前执行固定容量准入检查：

```text
external transport
    -> ExternalMotionCommand
    -> CommandIngress::admit()
    -> MotionPermit
    -> LifecycleGuard::accept_permit()
    -> CiA 402/profile policy
```

网关策略不执行 TLS、签名或用户会话认证。那些操作由具体的 Linux 传输适配器和可信监督服务完成；准入层只接受已经映射为固定身份、权限和策略版本的命令。

## 2. 固定命令契约

`ExternalMotionCommand` 包含：

| 字段 | 用途 |
| --- | --- |
| `boot_id` | 防止旧启动实例的命令进入当前系统 |
| `source_id` | 匹配固定大小的授权来源表 |
| `authority` | 与策略最低权限比较 |
| `permit_epoch` / `sequence` | 恢复代际与命令序号，拒绝回退和重放 |
| `deadline_ns` | 单调时钟 TTL 截止时间 |
| `axis_mask` | 限制命令可作用的轴集合 |
| `policy_version` | 绑定已激活的准入策略 |

通过后生成同等字段的 `MotionPermit`，实时域仍会再次验证 boot、来源、权限、策略版本、TTL、轴掩码和 epoch/sequence。双层检查使网关错误不会绕过 MLG。

## 3. 策略与审计

`IngressPolicy` 使用最多 4 个授权来源、允许轴掩码、最低权限、策略版本、最大 TTL 和固定窗口限流。容量或数值为零的策略不会扩大权限：时间和限流窗口归一化为最小有效值，来源表为空时拒绝所有命令。

每次准入结果都会写入最多 32 条的 `IngressAudit` 环，记录审计序号、单调时间、来源、permit epoch、命令序号、接受/拒绝决策和稳定错误码。环满时只覆盖最旧记录，不分配内存、不等待传输，也不向 EtherCAT 周期注入同步调用。

MLG 自身还保留 `PermitAudit`，记录实时边界再次拒绝的许可。监督域可以用两条审计环的序号、时间和 command/permit 序号建立因果关联。

## 4. 验证范围

当前公开测试覆盖：

1. 合法命令转换为 `MotionPermit` 并进入 MLG 的受控 rearm 路径。
2. 未授权来源、权限不足、策略版本错误、序号重放和限流拒绝。
3. 固定容量审计环的时间顺序与覆盖边界。

尚未声明完成的部分包括 Zenoh session/router、Protobuf 生成绑定、加密身份、具体 IPC、远程 ACL 配置和断连重连系统测试。`proto/esop/v1/esop.proto` 与 `esop-zenoh-gateway` 只作为版本化契约和路由边界，不改变本准入层的固定结构。
