# 验证方法与证据索引

用户操作见 [README](../../README.md)，当前职责与安全边界见[运行机制](../architecture/runtime.md)，命令、构建和发布合同见[开发指南](../development/build-and-release.md)。本页先说明如何验证，再保留关键历史结果；固定日期的通过不延伸到当前版本、其他平台或真实上游。

## 按变更选择验证

| 变更                               | 入口与边界                                                                                                                                                                                                                                               |
| ---------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Vue、状态与 typed IPC              | `npm run check`，页面流程按需运行 `npm run test:e2e`；合成 bridge 不代替 Tauri、系统凭据或平台网络                                                                                                                                                       |
| Rust/core/helper、持久化和配置事务 | [native-workflow](../../scripts/native-workflow.mjs)及受影响测试；预览失效、文件冲突、应用/恢复、迟到响应、幂等与恢复状态                                                                                                                                |
| 账户、API Key 与用量               | [Panel 夹具](../../tests/acceptance/panel/)、[真实消费者](../../crates/orbit-core/src/service/tests/panel_acceptance.rs)及[本地发布检查](../../crates/orbit-core/src/service/tests/release_check.rs)；独立数据库和内存 Vault 不代表系统 Vault 或真实钱包 |
| 图片网络与本地保存                 | [发布检查](../../crates/orbit-core/src/service/tests/release_check.rs)的独立合成链路，真实上游另走[显式入口](../../crates/orbit-core/src/service/tests/image_live_demo.rs)；草稿权限与提交资格分别检查，结果不确定不重发                                 |
| 托盘、窗口和文件系统               | 目标 OS 的真实 app、系统控件、普通用户令牌和实际窗口事件；浏览器 UA、纯几何测试、快捷键不代替真实托盘鼠标事件                                                                                                                                            |
| 安装、更新和发布                   | 双平台 package evidence、NSIS/DMG 与 updater 签名；包可创建、签名通过、安装/重启成功分别记账                                                                                                                                                             |

运行前固定完整源码、依赖锁、数据目录和实际包摘要，明确 peer、HTTP、Vault 与资金的替换组件。生产构建排除 demo/fixture；独立标识、数据库和非持久 WebView 的演示应用不证明正式数据迁移或登录链路。

以下外部入口保持显式 opt-in，具体保护以当前源码为准：

- [真实本地 Panel](../../crates/orbit-core/src/service/tests/panel_acceptance.rs)：受保护的独立 Panel/MySQL/TLS 与私有输入，包含 Key 管理和结果未知后的核对。
- [合成图片/只读用量](../../crates/orbit-core/src/service/tests/release_check.rs)：真实原生消费者与本地组件，合成会话/内存 Vault。证据目录以 create_new 防止同一用例原地重放。
- [真实图片上游](../../crates/orbit-core/src/service/tests/image_live_demo.rs)：当前明确预算与专属 fixture；重建目录不增加付款授权。
- [Codex/Keychain roundtrip](../../crates/orbit-core/src/service/tests/macos_config_roundtrip.rs)、[NativeVault](../../crates/orbit-core/tests/macos_native_preflight.rs)和 [CLI signer](../../crates/orbit-core/tests/updater_cli.rs)：各有独立 ignored 入口，不能计入普通合成通过。

未运行与 ignored 分开记录；历史预算、凭据及真实客户端写入授权不延续。Windows 真实多屏/DPI、全新 WebView2 引导和 macOS 完整签名包安装/在线更新等，须按实际候选另行提供证据。

## 本地平台与图片的历史结论

以下是当时的独立检查，具体测试数量和逐步失败通过固定 Git 版本追溯，不能据此免除后续变更的门禁。

- **2026-11-18；正式 v1.0.8 `54212e3d` → `acc3d4f4`**。完整前端、浏览器、锁定 Rust 与 Tauri 检查通过，外部验收仍单列。原生只读用量经严格 TLS/Ed25519、真实 Panel/MySQL 读取 12 条合成调用，保留两类缓存写入及未知值；共同付款父账户读到零条。首次用错会话归属与零汇总失败保留，真实 flush 恢复三个桶且钱包/账本不变；内存 Vault/预置会话不是真实登录

  证据：`.worktrees/preflight/evidence/orbit/`及 `runtime/orbit-usage/`。

- **2026-11-17；v1.0.7 `90f72237` → `2f658cef`**。三个原生 Service 用例实际经过 Panel/MySQL、Edge 和受控上游，默认订阅 PNG、API stream+n=2、蒙版编辑各一次；本地保存/解码及再次发送拒绝通过。正式 v1.0.7 源码旧 Service 另经 TLS 代理完成一次高级请求，不能当作旧 UI 默认订阅或正式包通过。首轮 CA 误作终端证书全部在 TLS 阶段拒绝，换合格证书后新证据根通过；无额外付费请求

  证据：原本地发布检查的 evidence/fixture，入口见 release_check.rs。

- **2026-11-17；图片准备/标题**。未选 Key/节点可准备草稿，提交仍核资格；同连接切页保留草稿，跨账户清空并丢弃迟到结果。本地请求 ID 标题/改名、旧记录读取、持久保存、宽窄布局和 Rust 回归通过；合成数据，不是实际图片消费

  证据：原候选 `.cache/evidence/`。

- **2026-11-17；模型同步 `39828b1` / Panel `c09f10df`**。支持推理但未给默认 effort 的模型仍进入兼容目录，Orbit 从权限交集选择值；绑定节点/Key、外部配置冲突、半发布与最初配置恢复有回归。后续通知收敛为刷新反馈和需处理的新故障；未取得当时 Windows 会话原始返回，也未做 Windows 原生或真实上游验证

  证据：原功能 worktree，逐项结果见整理前版本。

- **2026-11-16；v1.0.6 `5f2f4cbd` → `21a99b9d`**。实际 Rust → HTTPS 代理 → Panel/MySQL 验证显式选择、轮换、重授权、CSRF/PKCE/安装证明和登出。仅对指定合成创建断开成功回包，持久化请求核对恢复且未重复创建；旧 Caddy 本身不具备此注入。Weak 引用、TLS CA 和模型 slug 三次夹具失败保留；无系统 Vault/真实配置写入

  证据：Panel 验收夹具及原本地回执。

- **2026-11-16；API 图片与 Key 管理**。SVG/GIF 的解析预算、脚本/外部资源拒绝、媒体类型与 EOF 前流式交付、整批原始文件保存有合成回归，无真实图片或 Windows 原生请求。同日 Key 清理的独立 MySQL 8.4 并发验证旧客户端不能在清理后重新发行自动 Key，保留手动 Key、账务与安装 tombstone；未操作生产 Key

  证据：原功能 worktree，完整结果由整理前版本追溯。

- **2026-11-13；用量 `1611b791` / Panel `47db1d83`**。本人全部入口、多币种、未知费用、时区/筛选/分页、只读与迟到响应回归；图片统一缩略图/原比例放大和模型显示名兼容通过。旧参数复用已移除。仅合成 UI/Rust/Panel 检查，不是当时 MySQL 或真实平台验收

  证据：原 usage/images 报告。

- **发布复查；v1.0.3 `33a5e56d` → `d419fe4c`**。当轮 quality、浏览器、core/helper/Tauri 通过；工作台 Prettier 修于 `5978798`。Chrome 沙箱、缺临时目录的首轮准备失败和外部 ignored 保留；旧 Windows/DMG 回执不能替代此候选的签名包生命周期

  证据：原 release-gate 报告。

v1.0.8 的 origin 同名标签曾与 GitHub 正式标签存在一条浏览器 matcher 差异，产品树一致；原检查未移动标签。上表不声明今天最新发布状态，也不代表远端 CI 或生产部署。

## 桌面与安装包证据

机器回执在[验收资料索引](../../tests/acceptance/README.md)中保留原样。下面集中记录已修复失败、真实操作范围与欠缺项。

- **2026-11-11；macOS arm64**。隔离 app 的托盘创建、关闭隐藏、单实例、菜单导航和退出保护通过；未直接鼠标点击图标、未证明真实 Claude 后台流量连续

- **2026-11-12；Windows v1.0.1 → 候选 v1.0.2**。真实 NSIS、Windows 凭据、旧设置/图片历史、托盘/对话框/剪贴板/保存文件和退出保护；远端为受控 HTTPS peer。初版图标生命周期编译失败修复后通过；旧参数复用只描述原版本

- **2026-11-12；DMG**。实际 verify、只读挂载、Applications 链接、背景精确摘要和 Finder 布局；不含用户实际安装、重启或在线更新

- **2026-11-13；配置格式**。Codex/Claude 只读 preflight 及合成恢复通过，改以配置能力判断而非精确版本/文件指纹；预览仍在应用更新后失效，不证明未来格式或 Windows 写入

- **2026-11-14；macOS 快捷面板**。原普通 NSWindow 跨 Chrome 全屏点击不可见，追加 collection behavior 仍失败；改 NSPanel/NonactivatingPanel 并去掉激活整个 app 的 set_focus 后，在用户真实单屏点击下原地展开。仅展示期间注册外部鼠标监听，点击 Chrome 后收起、重复图标切换及原失焦路径通过；多屏按当时用户要求暂缓

- **2026-11-14；左右键统一**。左右键改为同一快捷面板，创建失败保留备用菜单。macOS 隔离快捷键、菜单/退出及编译通过，但该次真实图标左右键/全屏/多屏读取超时，未计通过；前一轮左键成功不覆盖新右键

- **2026-11-15；Windows 原生客户端**。真实 MSIX Codex 与 Claude MSIX → 完整 EXE 重装均经 Windows Credential Manager/helper 和默认文件路径请求受控 HTTPS peer，随后恢复/撤销。单屏已有 WebView2，不覆盖多屏或新系统引导；未签名内部候选不代表正式发布

- **2026-11-16；托盘初始焦点**。共享前端回归覆盖打开无默认按钮、Enter 不导航、Tab 正常轮廓及重开。真实 macOS AppKit NSPanel/WKWebView 补验修复隐藏后残留轮廓；未构建完整 Tauri、未点真实图标，Windows 仅脚本/浏览器检查

macOS NSPanel 最终单屏成功与后续左右键缺少实测分别保留，不能用 `visible=true`、快捷键或几何测试代替用户看到面板。原生事件与焦点 JSON 绑定各自候选，不是当前多屏通过证明。

Windows 最终普通用户交付由 `windows-native-2026-11-15.json` 的 `current_delivery` 与 `follow_up_ordinary_user` 读取，绑定 `b4def7f408a9a29ec1c2098770dd4866a2237714`。WindowsApps 父目录访问改用逐层元数据句柄，导出不再为已有当前所有者多申请 WRITE_OWNER；身份、签名、重解析和私有权限保护保留，并有真实普通用户检查。提升权限的环境准备/输入进程不充当应用运行条件。

同轮 Codex 恢复曾在 publishing 被安装校验拒绝，触发原因未确定；原生 journal 重启续恢复、旧 helper 拒绝与后续最终复验分别保留。初始托盘误点 OneDrive、页面选择器和 fixture 缺路由也保留为失败。原黑窗归属事后未捕获，验收启动器改 GUI/隐藏进程后连续控制台 SHOW 观察为零，不据此抹除原反馈。

普通用户 Windows 入口为 `tests/windows/with-panel-fixture.ps1 -OrdinaryUser`，限明确标记的 disposable VM。按现有构建流程设置被测程序与 RUNNER_TEMP，将 `tests/windows/hidden-test-host.cs` 用已有 .NET Framework 编译为 `runtime/hidden-test-host.exe`（/target:winexe /platform:x64）；测试启动器和观察器不入发行物。

### Windows 应用内更新（2026-11-15）

[Actions 实装验收](https://github.com/Morton-Li/Orivelle-Orbit/actions/runs/43730209076)完成真实签名 NSIS 0.2.0 → 0.3.0、确认后安装与自动重启；检查错误包/版本签名、下载取消、维护互斥和安装前篡改。语言、主题、启动检查及既有数据保留，新程序/helper 摘要匹配回执。

包来自[构建运行](https://github.com/Morton-Li/Orivelle-Orbit/actions/runs/43727014430)的 `425bc43`；最终断言脚本 `1bc6112` 只改对话框断言，经祖先/输入/摘要/签名核对后复用包，没有声称重编。使用临时 updater 密钥、受控 HTTPS peer 和已有 WebView2，没有正式 Release。Windows 使用 NSIS 错误处理，不具有已验收的 macOS 式数据库/app 自动回滚。

### 真实图片样本（2026-11-11）

Orbit `5f8ae5d`、Panel `c420b919`、Edge `8b74d79c` 在当时三张预算内仅发送一张生成和一张编辑，均 n=1、无自动重试。实际原生 Service 经 Docker Panel 和正式 Edge 代码到上游，但使用合成会话、FakeVault、独立 SQLite 钱包及内存凭据 broker，不覆盖桌面登录、MySQL 并发或系统 Vault。

| 项目               | 生成                                                               | 编辑                                                               |
| ------------------ | ------------------------------------------------------------------ | ------------------------------------------------------------------ |
| 请求               | 1024x1024、low、opaque、PNG                                        | 1536x1024、low、opaque、JPEG/65                                    |
| 完整解码的实际文件 | 1536×1024 PNG                                                      | 1536×1024 PNG                                                      |
| SHA-256            | `e3d53041ca008c9df047e30af72ed3155cf8a71b6b1e232d7375026e40a8a132` | `39d4ea4b8b160f9ddf5f7c0fbc3cddf14540e6b24d32270125bc208c29b0c7d9` |
| 合成销售金额       | USD 0.01                                                           | USD 0.01                                                           |

每次只有一个 attempt/终态/permit，按 image_count 合成结算并释放租约。未知实际模型与可选 HTTP 状态没有补成已观测 200；请求参数与文件差异说明接受不等于兑现。首次独占性检查与可选状态断言失败没有增加上游请求。收尾撤销合成 key、停止 broker/demo 模型；旧预览地址不是现行服务。安全摘要原位于旧 worktree `.cache/live-demo/evidence/acceptance-summary.json`，不保证仍在磁盘。

## 历史追溯与维护

`ea39836` 保留本轮整理前原文：

```sh
git show ea39836:docs/testing/validation.md
git show ea39836:tests/acceptance/desktop/menubar.md
git show ea39836:tests/acceptance/desktop/windows-tray.md
git show ea39836:tests/acceptance/desktop/tray-focus.md
```

更早 release-gate、desktop/images/usage 阶段报告可从 `63631f2` 对应旧路径查询。可执行夹具与机器 JSON 不删除、不改写；原生包只允许固定 Tauri UNK→NSS 标记差异，不能泛化任意摘要差异。

新增证据记录完整源码、平台/依赖、包摘要、替换组件、实际命令、失败/跳过/未运行及完成条件；只将稳定方法、独有失败与剩余条件合入本页，逐轮日志/截图/私有输入留在证据目录。
