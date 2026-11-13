# 2026-11-13 本地上线前检查

正式稳定基线为 `v1.0.3`（`33a5e56dffe0a27c86c6b3e80a6cc554a1b96f59`），本地 develop 候选为 `dd34488c170e57da0c3f08b006acf288b10ba482`。远端 develop 为 `5c6d537b8ebd106c1f2dcc0aacf480cf463e306c`；候选包含后续待发布修改。tag 与共同祖先产品树相同，main 纯合并历史没有遗漏产品树。

范围为 52 个变更文件及相关旧入口：Codex/Claude 配置兼容与预览失效、只读用量 HTTP/Rust/IPC/UI、更新下载进度/取消/重试和安装确认。确认的本轮门禁阻塞为用量验收 README 表格不符合 Prettier，已在 `39ac714d8801f7179f67e62c3274a4b9726ac293` 最小修复。未确认其他新增业务阻塞。分支为 `feature/release-gate`；报告提交不改变已验证代码，最终完整 HEAD 见任务 `evidence/final-state.json`。

## 必要验证

| 验证 | 结果 |
| --- | --- |
| 完整 npm check | format、lint、typecheck、163 单测、build、生产夹具排除全部通过 |
| 构建脚本组 | 83/83 通过 |
| Playwright 完整工作流 | 30/30 通过，含只读用量、旧入口、下载取消/重试/确认安装 |
| Rust core/helper | core 278 通过，3 原有外部验收 ignored；helper 2/2 通过 |
| 其他 Rust/native 检查 | fmt、typed Tauri 通过；真实 Keychain、CLI signer 的独立验收各 1 ignored |
| 锁依赖 | 266 个已安装包均符合 lock，无版本漂移或缺失必需依赖 |

第一次浏览器执行在页面断言之前被沙箱 EPERM/SIGABRT 阻止；允许原生 Chrome 启动后，同一完整入口通过，没有改断言。实际浅色/深色用量页截图保存在任务 Orbit `.cache/usage-demo/usage-light.png` 和 `usage-dark.png`，浅色布局已实际查看。Rust 首次离线 cache 缺失改用项目已有缓存后完成，没有安装系统依赖。

独立复核沿网络、Rust 数据消费者、IPC 和页面核对：旧 Panel summary 404 明确映射不可用状态；账户/会话版本检查阻止迟到响应混入其他账户；只读查询不要求推理 key；费用币种、未知和待确认状态分别保留；更新取消槽位与重试、预览时应用变化、旧配置恢复继续保持保护。

## 实际组件与兼容边界

另用真实本地 Panel、隔离 MySQL、生产 Edge main 和受控 SSE 上游产生 5 条合成执行（4 成功/1 取消）。独立签名 HTTP 客户端经实际 PKCE/安装证明后以只读会话请求 summary/records，均返回相符的 5 条；查询不改余额/结算，旧 Panel 缺 summary 为 404。后续会话继续验证由旧 Panel 代码在已升级合成库建立，再切换新 Panel 代码读取。此网络部分没有经过原生 Orbit app/IPC/Keychain；本仓 Rust、IPC/前端工作流在上表各自入口验证，不能合称原生 app 全链路验收。

用量是只读展示，不改计费或预算语义；旧 Panel 的新汇总入口缺失被明确呈现。配置兼容继续检查应用身份、当前格式和预览变化，不要求固定桌面版本指纹。更新改动涉及下载/取消/展示；签名和事务安装/恢复实现未变，完整 Rust 组保留隔离文件事务安装及失败回退覆盖。

没有执行真实稳定版 `.app` 在线升级安装/重启/卸载、Windows 原生、真实桌面配置写入、真实 Keychain/CLI signer、模型上游或生产操作；浏览器“确认安装”验证桥调用，不代表实际安装包升级。也没有执行远端 CI。

未参与实现的独立 reviewer 已核对正式差异、最终格式修复和验证证据；最终结论见任务 `evidence/independent-review.json`。原始日志保存在项目外层 `.worktrees/release-gate/evidence/orbit/`，网络证据在 `integration/oauth/result.json` 与 `integration/continuation/result.json`。本任务只本地提交，不推送、合并、改 tag、发布或部署。
