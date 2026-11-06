# Claude Desktop 本地配置库

Orbit 的自动保存与恢复适用于 macOS arm64 的 Claude Desktop `4.4212.0`，bundle ID 为 `com.anthropic.claudefordesktop`。第三方模式作用于整个 Claude Desktop，包括 Chat、Code 和 Cowork，保存后需完整重启 Claude。 [桌面接入配置](desktop-configuration.md) · [Claude 配置参考](https://claude.com/docs/third-party/claude-desktop/configuration)

## 配置文件与来源

三个文件均位于 `~/Library/Application Support/Claude-3p/`，按下表顺序写入并读回。

| 顺序 | 相对路径 | 变更 |
| --- | --- | --- |
| 1 | `configLibrary/<新 UUID>.json` | 新建扁平的 Orivelle profile |
| 2 | `configLibrary/_meta.json` | 保留既有条目和其他字段，追加新条目并将 `appliedId` 指向它 |
| 3 | `claude_desktop_config.json` | 将 `deploymentMode` 设为 `3p`，保留其他字段 |

新 profile 继承当前选中 profile 中明确允许保留的设置，并替换已知的推理连接、凭据和模型字段。原 profile 作为只读依赖参与检查。预览遇到损坏索引、所选文件缺失、`hybridPointer`、当前 profile 由 bootstrap 提供或出现无法分类的设置时停止。只有索引原本缺失时才从空配置开始。已存在的 `coworkEgressAllowedHosts` 通过有界主机模式校验后按原样继承。

Claude 可在启动时封装旧明文凭据并清理旧主配置字段。完整资格检查同时扫描配置库中的未选中和孤立 profile。若显示 `claude_native_migration_required`，请先由 Claude 完成自己的迁移并退出，再生成新预览。Orbit 的检查只识别迁移状态。 [配置编译器](../crates/orbit-core/src/claude_config.rs) · [安装核验](../crates/orbit-core/src/system.rs)

V2 格式导出供诊断审阅，导出入口会重验资格、上下文及目标路径。实际应用和恢复通过 Orbit 的预览与确认流程进行。 [导出实现](../crates/orbit-core/src/service.rs)

## 凭据与本地模型映射

新 profile 通过 helper 路径、模式和绑定标识配置凭据，helper 预算为 60 秒。读取旧加密记录时，事务验证接受完整文件摘要精确匹配的旧 5 秒输出。直连 profile 使用 Panel 授权的 `claude_base_url` 节点根地址，由 Claude 追加 `/v1/messages`。 [Claude 凭据 helper](https://claude.com/docs/third-party/claude-desktop/credential-helper)

所选模型 ID 以 `claude-` 或 `anthropic/claude-` 开头时使用直连。其他模型通过本地映射，将 Sonnet、Opus、Haiku 角色统一路由到所选模型。预览显示实际模型和本地地址。映射只在 `127.0.0.1` 上提供该绑定的认证模型目录与 Messages，替换请求的顶层模型字段，并转发上游流式或工具响应。权限、预算和账务以 Panel 对实际模型的记录为准。 [本地转发实现](../crates/orbit-core/src/claude_mapping_http.rs)

映射模式的 helper 只返回该绑定的本地 token，真实推理 key 由原生转发器从系统安全存储读取并用于冻结的 HTTPS 节点。macOS 的新 token 条目绑定固定的 Orbit 与 helper 代码身份，并将 helper 权限限定为解密。创建时核对条目范围及持久化权限，核对失败时清理本次新建条目。锁定的钥匙串仍可能需要解锁，实际 helper 读取和升级后的访问连续性需原生验证。 [钥匙串访问实现](../crates/orbit-core/src/vault/macos_mapping_access.rs) · [验证范围](validation.md)

映射在配置提交并启用绑定后提供服务，需要 Orbit 进程保持运行。退出进程会停止映射，重开会尝试重新绑定配置中的原端口，端口冲突会报错。每次本地请求最多转发一次上游，取消或断连后的上游执行与费用以 Panel 可核对的状态为准。

## 恢复 Claude 配置

默认恢复逐字节核对日志中的预期文件状态，按主配置、索引、新 profile 的反向顺序撤销变更。若 Claude 在使用后更新了 `preferences`，可对未到期的已提交备份选择「保留偏好后预览恢复」。此模式在原主配置存在、且当前除 `preferences` 外的所有顶层键及原始值仍匹配保存结果时可用。确认后保留现有偏好，将 `deploymentMode` 恢复为原值或缺省状态，并按原字节恢复索引和移除新 profile。

偏好恢复预览冻结当前完整字节及恢复目标。确认前再次发生修改时，需要重新预览。应用和恢复新写入的文件均以 `0600` 权限发布。恢复完成后重启 Claude，实际载入和连接另行验证。 [恢复实现](../crates/orbit-core/src/claude_config.rs) · [验证范围](validation.md)
