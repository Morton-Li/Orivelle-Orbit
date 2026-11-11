# 菜单栏与系统托盘验收

2026-11-11 的原生验收范围为本机 macOS arm64；该轮未进行 Windows 构建或原生运行验收。2026-11-12 补充的 Windows 与 DMG 验收记录见本文后续章节。

## 已完成的检查

- 前端类型检查、lint、格式检查、生产构建和产物测试夹具排除检查通过；134 项前端单元测试通过。
- 关闭行为的浏览器回归通过：两平台均可选择收起或退出，旧 `platform_default` 设置显示为收起，并可保存和切换。
- Rust 关闭行为设置回归通过，覆盖新默认值、旧设置兼容、退出选项以及非法值不写入。
- macOS Rust 编译与隔离 `.app` 构建通过。隔离运行未启用 `system-integration`，使用项目内数据目录与非持久化 WebView。
- 原生托盘已创建，Tauri 返回有效的图标矩形。
- 原生关闭事件后 `is_visible()` 返回 `false`；随后既有单实例监听器成功接收激活请求并恢复窗口，未重新启动服务。
- 从原生应用菜单选择“设置”后，已有窗口进入设置路由。应用菜单与托盘共用同一菜单处理器。
- 使用仅写入隔离数据库的合成待处理任务验证退出保护：`⌘Q` 弹出确认，选择“继续运行”保留窗口与任务，关闭窗口后任务仍为待处理状态。
- 再次选择退出并确认后，单实例监听器释放。合成任务已清理，隔离设置已恢复。

macOS 的预定义 Quit 菜单直接调用 `NSApplication.terminate`，不会经过 Tauri 的 `ExitRequested`。实现使用自定义原生退出菜单，让应用菜单、`⌘Q`、托盘退出和“关闭时退出”进入同一检查流程。程序化退出及更新重启继续放行。

## 验收边界

- 界面工具无法直接读取系统菜单栏额外图标；菜单动作通过原生应用菜单验证共用处理器，未单独自动化鼠标点击菜单栏图标。
- 合成待处理任务用于检查退出保护与状态保留，没有发送付费图片请求，也没有验证真实 Claude 转发流量在隐藏窗口期间的连续性。
- 未安装真实更新或执行系统注销；更新重启的放行逻辑经过源码检查，未作为真实升级验收结果。
- Windows 原生构建、托盘鼠标行为、任务栏恢复以及安装包行为待后续 Windows 环境验收。

## 后续原生复核步骤

1. 正常启动，确认图标在 macOS 菜单栏或 Windows 系统托盘显示，主窗口正常打开。
2. 关闭窗口，确认进程仍运行；macOS Dock 图标隐藏，Windows 任务栏窗口入口隐藏。
3. 从常驻图标打开窗口、进入设置；再次启动应用，应唤回已有窗口。
4. 在进行中的图片或 Claude 请求期间收起窗口，确认处理继续；从 Orbit 菜单退出时核对确认提示及两个按钮的行为。
5. 将关闭行为改为退出，确认关闭窗口走真正退出流程；安装更新时另行验证重启。

## 2026-11-12 Windows 与 DMG 验收

原 v1.0.2 的 [Release Orbit 运行](https://github.com/Morton-Li/Orivelle-Orbit/actions/runs/43042209337) 在 Windows 编译失败：默认图标的 `clone()` 保留借用生命周期，无法作为 `Image<'static>` 返回。修复使用 `clone().to_owned()`，Windows 原生编译与打包随后通过；本次同时删除 `.gitlab-ci.yml`。

### Windows 回归与真实安装升级

- [Windows 回归任务](https://github.com/Morton-Li/Orivelle-Orbit/actions/runs/43064366241/job/130675218643) 通过：134 项前端单元测试、22 项浏览器工作流、134 项 Rust 核心测试、2 项 helper 测试和 67 项构建脚本测试。付费实时 demo 与 POSIX 专用测试各有一项按平台或范围跳过。
- [Windows 原生验收](https://github.com/Morton-Li/Orivelle-Orbit/actions/runs/43077671970) 通过。先下载已发布 v1.0.1 的 NSIS 安装包并核对官方 `SHA256SUMS`，实际安装、登录并创建旧参数图片历史，再由候选 v1.0.2 NSIS 安装包升级同一目录。
- 升级后保留 Windows 凭据存储中的会话、账户、活动连接、深色主题、旧 `platform_default` 关闭设置及图片历史。
- 原生关闭窗口保留进程；再次启动退出次实例并唤回主窗口。真实鼠标打开托盘菜单，按 MSAA 名称与位置点击“设置”；关闭选项可切换并持久化。
- 新任务默认显式提交 `background: opaque`，编辑任务显式提交 `transparent`；高级选项默认收起，折叠摘要不重复展示参数，未证实支持的旧参数控件不出现。旧任务的 `1024x1024`、高质量及透明背景事实保留，复用时仅保留当前支持的参数。
- 受控 HTTPS 图片响应保持未完成期间，收起窗口后从原生托盘退出，真实原生确认框弹出；点击“继续运行”后再次收起，任务继续完成，随后真实托盘双击恢复窗口。
- WebView2 图片预览、Windows 图片剪贴板、真实保存/打开文件对话框、导出字节一致性、参考图导入、编辑、历史删除、服务端用量对账及注销撤销通过。主题、语言、Windows 更新禁用状态及诊断报告真实保存通过；显式“关闭时退出”以退出码 0 结束主进程。

原生验收脚本提交为 `99b2eb4`，复用的已编译应用来自 `3c64e7f` / Actions `43069688256`。工作流检查同仓库祖先关系、应用构建源码未变、构建清单与可执行文件 SHA-256；后续差异仅为验收脚本和工作流。详细回执保存在 [v1.0.2-windows.json](v1.0.2-windows.json)，原生截图保留在上述 Actions 的 `orbit-windows-integration` artifact 中。

NSIS 负载与实际安装文件均按固定版本 Tauri 的规则校验：仅允许 `__TAURI_BUNDLE_TYPE_VAR_UNK` 变为 `__TAURI_BUNDLE_TYPE_VAR_NSS`，其余字节必须完全相同。该变化是 [Tauri bundler 的包类型标记](https://github.com/tauri-apps/tauri/blob/tauri-cli-v2.12.2/crates/tauri-bundler/src/bundle.rs)，并非放宽任意哈希差异；单元测试覆盖正确转换以及代码、资源、长度、重复或缺失标记的拒绝路径。

### macOS DMG

参考 Aplena 的拖拽安装布局，使用 Orbit 自身的绿色品牌背景、拖拽箭头和安装提示。Finder 窗口为 660 × 420 点，背景为 2 倍像素资源，应用与 Applications 图标固定在 `(180, 220)` 和 `(480, 220)`。

[DMG 原生验收](https://github.com/Morton-Li/Orivelle-Orbit/actions/runs/43070439297) 通过：实际 `hdiutil verify`、只读挂载、应用目录、`/Applications` 符号链接、`.DS_Store`、背景文件精确 SHA-256、Finder 图标位置、窗口尺寸及实际挂载路径均通过。原生 Finder 截图已人工查看。回执见 [v1.0.2-dmg.json](v1.0.2-dmg.json)，截图在 `orbit-macos-dmg-layout` artifact 中；正式发布工作流也执行同样的挂载验证并上传 `orbit-dmg-layout`。

### 本轮边界

Windows 使用真实 NSIS、原生可执行文件、WebView2 和系统控件；Panel、图片与用量响应来自临时 runner 上的受控 HTTPS 服务。没有新增付费上游请求，也不将这些结果视为真实第三方模型、Claude/Codex 桌面客户端配置应用或真实流量验收。Windows 自动更新按现有产品策略保持禁用；未执行真实更新安装、系统注销或关机测试。前述 2026-11-11 macOS 原生行为的证据范围保持原样。
