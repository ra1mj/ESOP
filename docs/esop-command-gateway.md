# ESOP 外部命令准入层

- 文档版本：1.0
- 日期：2026-09-09
- 状态：固定策略、permit 转换和 Zenoh/Protobuf 命令桥接已实现；传输安全配置准入已实现，加密身份映射、远程 ACL 和生产部署仍待集成
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

网关策略不执行 TLS、签名或用户会话认证。Zenoh 运行时适配器提供 `open_secure` 配置准入，确保生产 Session 的安全参数显式存在；实际证书有效性、签名身份映射、密钥轮换和远程 ACL 仍由具体的 Linux 传输适配器、可信监督服务和部署环境完成。准入层只接受已经映射为固定身份、权限和策略版本的命令。

Zenoh 适配器的 `admit_authenticated_command` 会比较可信监督服务提供的认证主体映射与 Protobuf 中的 `source_id`。映射不一致的命令在固定准入前拒绝；这只是身份绑定边界，不替代 TLS、签名、密钥轮换或远程 ACL。

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

尚未声明完成的部分包括加密身份、具体 IPC、远程 ACL 配置和生产断连重连测试。`proto/esop/v1/esop.proto` 与 `esop-zenoh-gateway` 已提供版本化契约、真实 loopback router 验证和受控命令入口，不改变本准入层的固定结构。
