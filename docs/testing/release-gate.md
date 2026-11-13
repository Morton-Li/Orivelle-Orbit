# 2026-11-13 Orbit 单仓上线前复查

正式稳定基线仍为 `v1.0.3`（`33a5e56dffe0a27c86c6b3e80a6cc554a1b96f59`，tag 对象 `ae2fc9fa4a6a465735ab5831229db94859b9ed71`）。本次候选为本地 develop `d419fe4c21bfde94324ce5dca06339a775dbc209`，包含图片工作台及用量错误布局两条最新调整；远端 develop 为 `66e4bcd415f2c21155435e1293b5c2a83c9cb71f`。tag 与共同祖先产品树相同，没有用 main 替代正式版本基线。

正式版到候选共 60 个变更文件；与上一轮已验收 `08ec0a78b2d6ac381a8b2dc0e64ede22e27d8f87` 相比新增 23 个变更文件。仅复查 Orbit，不修改或重测 Panel、Edge。任务 worktree 为 `.worktrees/orbit-preflight/Orbit`，分支为 `feature/orbit-preflight`；归档后的完整 HEAD 见任务 `evidence/final-state.json`。

## 本轮结论与修复

唯一确认的新增门禁阻塞是 `tests/acceptance/images/workbench.md` 未通过完整 npm check 的 Prettier。已在 `59787986157bccea2b07cb0a6c9f1bba62f2a4c1` 最小格式修复，没有修改产品语义或放宽断言。未发现其他确认的新增业务阻塞。

已核对图片页面、TypeScript IPC、Tauri 控制器/权限和 Rust 数据消费者：模型显示名在创建时与任务/快照持久化；旧 JSON 缺字段时兼容；请求上游仍使用原调用模型 ID，不发送显示名。SQLite schema 未变，真实临时 SQLite 测试覆盖名称、目录移除和旧记录读取。已批准移除的参数复用入口从 UI、合同、Rust 方法、命令注册和权限闭包完整移除；再次编辑仍使用历史图片作为参考图，取消、本地重存、身份隔离及原有账务字段继续保留。

结果与放大预览保持图片原始比例，横/方/竖历史缩略图使用统一预览框。用量错误提示间距、右侧刷新按钮和窄窗口重试均经实际浏览器验证，独立 reviewer 已查看截图。

## 本轮实际执行

| 验证 | 结果 |
| --- | --- |
| 完整 npm check | format、lint、typecheck、162 单测、build、生产夹具排除全部通过 |
| Playwright 完整组 | 34/34 通过，含图片比例、名称兼容、历史操作、用量错误和重试 |
| Node 构建脚本完整组 | 83/83 通过 |
| native-workflow preflight/check | Rust fmt、core 279 通过/3 既有 ignored、helper 2/2、typed Tauri 全部通过 |
| 原生命令闭包 | build、permissions.allow、invoke 注册的 31 个命令一致，无复用命令/权限残留 |
| Windows 脚本 | 语法检查通过，未作 Windows 原生验收 |
| 依赖 | 266 个本平台已安装包精确匹配 lock；包锁/Cargo.lock 未改，原生使用已有离线 locked 缓存 |

首次浏览器执行在页面断言前被沙箱 EPERM/SIGABRT 阻止；允许原生 Chrome 启动后同入口完整通过。构建组首轮有 14 个 ENOENT，根因为任务 TMPDIR 尚未创建；环境就绪后完整组通过。原失败、修正环境后的日志分开保留，不能写成首轮全绿。没有安装依赖、接受系统许可、改宿主设置或写真实客户端配置。

## 证据复用与未跑边界

网络、用量服务、配置兼容、系统安装和更新下载实现相对上一轮未变；已逐文件确认。其旧 Panel 404 降级、签名 HTTP 与 Panel/Edge 合成联调、取消/幂等，以及隔离文件事务安装/回退证据引用前轮精确候选，不宣称本轮重跑或原生 app 全链路。之前的报告可从 `git show 08ec0a78b2d6ac381a8b2dc0e64ede22e27d8f87:docs/testing/release-gate.md` 追溯，原始证据在外层 `.worktrees/release-gate/evidence/`。

本轮没有 Panel/MySQL 迁移、接口/协议或安装升级实现变更，故不重跑这些未变化链路。Rust 的 3 个 core 外部验收保持 ignored：本地 Docker 图片预算验收、真实 Codex 默认配置/Keychain roundtrip、真实本地 Panel PKCE；另有真实 Keychain 和 CLI signer 独立验收各 1 ignored，均未计为通过。未执行真实上游/凭据、真实图片消费、原生 Orbit 网络全链路、Windows 原生、正式签名 `.app` 实机在线升级/安装/重启/卸载、远端 CI 或生产操作。

本轮证据位于项目外层 `.worktrees/orbit-preflight/evidence/`：`baseline.json`、`dependency-lock.json`、`ui/report.json`、`native/report.json`、`independent-review.json` 和 `final-state.json`。实际截图为 `ui/images-history-uniform.png`、`ui/usage-error-1280.png`、`ui/usage-error-460.png`。独立 reviewer 未参与实现，最终核对范围、修复 diff、验证证据及上述限制。

仅提交本任务修改，保留任务分支并在同一 HEAD clean detach；不推送、合并、改 tag、发布或部署。
