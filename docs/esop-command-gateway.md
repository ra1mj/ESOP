# ESOP 外部命令准入层

- 文档版本：1.0
- 日期：2026-09-26
- 状态：固定策略、permit 转换、共享 target 结构验证、ProcBuf v6 命令页交付和 Zenoh/Protobuf 命令桥接已实现；传输安全配置准入已实现，加密身份映射、远程 ACL 和生产部署仍待集成
- 上游需求：PRD FR-031、FR-044、FR-045、NFR-014

## 1. 边界

外部网络、ROS 2、Zenoh 或维护接口不能直接写入实时 Command page，也不能直接修改 CiA 402 controlword。`esop-command-gateway` 位于监督域，负责在进入实时边界前执行固定容量准入检查：

```text
external transport
    -> MotionCommand target validation
    -> PreparedProcBufCommand
    -> CommandIngress::admit()
    -> MotionPermit + AdmittedProcBufCommand
    -> retryable ProcBuf CommandPage publication
    -> RT permit reconstruction
    -> LifecycleGuard::accept_permit()
    -> CiA 402/profile policy
```

网关策略不执行 TLS、签名或用户会话认证。Zenoh 运行时适配器提供 `open_secure` 配置准入，确保生产 Session 的安全参数显式存在；实际证书有效性、签名身份映射、密钥轮换和远程 ACL 仍由具体的 Linux 传输适配器、可信监督服务和部署环境完成。准入层只接受已经映射为固定身份、权限和策略版本的命令。

Zenoh 适配器的 `admit_authenticated_command` 会比较可信监督服务提供的认证主体映射与 Protobuf 中的 `source_id`。映射不一致的命令在固定准入前拒绝；这只是身份绑定边界，不替代 TLS、签名、密钥轮换或远程 ACL。

需要写入实时命令页的调用方使用 `esop-ipc/payloads` 的严格路径。它在调用 `CommandIngress` 前验证 CSP/CSV/CST、轴容量与掩码、零基轴索引的一一覆盖、有限值和非负限值；通过后只从返回的 `MotionPermit` 填充权限字段，并把结果绑定到目标 ProcBuf 的 robot、boot、layout 与容量。发布借用已准入对象，因此失败重试不会再次消耗 replay/rate-limit 状态。Zenoh 的 ProcBuf 命令方法委托同一映射器。

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
4. 所有 target 结构错误在 ingress 状态变化前拒绝，准入字段原样进入 ProcBuf ABI v6。
5. Unix datagram 到 ProcBuf readback、permit 重建和 MLG 接受的完整软件路径，以及错误目标 buffer 后从同一已准入对象重试发布。

尚未声明完成的部分包括加密身份、远程 ACL 配置、产品机械限位、PDO 缩放、真实驱动执行、生产断连重连性能和实物 HIL。`proto/esop/v1/esop.proto` 与 `esop-zenoh-gateway` 已提供版本化契约、真实 loopback router 验证和受控命令入口；目标结构验证不能替代 RT profile 与设备资格。
