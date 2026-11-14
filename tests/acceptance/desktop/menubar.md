# macOS 菜单栏快捷面板

验证日期：2026-11-14。实现分支：`feature/macos-menubar-panel`。

## 实现范围

- macOS 左键展开独立快捷面板，右键保留原生菜单；面板入口定位到主窗口对应页面。
- 展示账户余额或预算、今日全平台已确认消费、桌面接入与图片任务；不同币种分开展示，未确认金额保留原状态。
- 面板和主窗口共用主题逻辑，跟随 Orbit 的浅色、深色、跟随系统设置与语言设置。
- 收起面板保留后端服务和任务；收起后停止面板的用量轮询。退出继续使用现有任务保护。
- 面板使用独立权限，只允许读取汇总、刷新账户和调用受约束的窗口控制，不具备配置客户端或提交图片任务的权限。

## 自动化与编译

- `npm run check`：格式、lint、类型、单元测试、生产构建和测试夹具排除检查通过。
- 全部浏览器回归：35 项通过，包含主窗口原有流程与新增面板测试。
- 新增面板测试覆盖固定主题与系统主题切换、账户切换后丢弃旧结果、跨币种与大数金额、隐藏后停止轮询、导航和排队阶段。
- `cargo check --locked --offline -p orivelle-orbit` 与桌面库测试通过；几何测试覆盖屏幕边缘、负坐标显示器与较矮的工作区，IPC 测试覆盖受约束的导航目标与拒绝额外字段。
- 本地 Rust 验证使用已有的 1.98.0 编译器；项目固定的 1.97.1 未改动。本次没有执行要求精确 1.97.1 的正式发布流程。

## 原生实例与验证限制

已在项目 `runtime/menubar-smoke/` 构建独立测试实例，使用专用 identifier、项目内数据目录、临时 WebView 存储与不可用的凭据存储。未连接真实账户。首次实例可正常启动，可打开设置并切换主题。续验完成后已退出测试实例，将 Chrome 恢复为普通窗口并关闭本次创建的空白验收标签页。

首次实现时，桌面自动化读取系统菜单栏超时，原生图标点击、失焦、全屏和多显示器没有记为通过。后续原生续验结果见下文；浏览器夹具与纯几何测试不能替代原生验收。

本次不包含安装到“应用程序”、系统配置变更、推送、发布或更新已安装的 Orbit。

## 2026-11-14 原生续验

当前只有一块在线的 LG ULTRAFINE 显示器，物理分辨率 3840×2160、逻辑分辨率 1920×1080、缩放 2 倍。用户明确选择先验收现有屏幕，多屏接屏后再补测。

增加了默认关闭的 `native-acceptance` 构建特性，仅在专用测试 identifier 下提供原生验收菜单与最多 256 KiB 的本地窗口事件记录。普通构建没有该菜单或事件记录。原生菜单的 `⌘⇧P` 入口调用生产面板相同的展开/收起逻辑，但它不作为真实状态栏鼠标点击通过的证据。

已实测通过：

- 原生面板真正展开、获得焦点，实际 WKWebView 内容与效果稿一致。
- 在面板外点击测试主窗口，收到原生失焦事件，随后面板 `visible=false`。
- Orbit 测试主窗口进入原生全屏，主窗口 `fullscreen=true` 时面板仍能展开并获得焦点。
- 全屏中按 `Esc`，面板隐藏，主窗口继续保持全屏。
- 普通构建与开启验收特性的构建，各自的 5 项桌面库测试通过。

初版真实状态栏点击已得到事件记录：用户在 Chrome 全屏场景点击后，原生进程记录了 4 次 `tray-left-mouse-up`，并进入展开流程。但用户反馈没有看到面板，因此初版该场景判为失败，不能把 `visible=true` 或取得焦点当成视觉验收通过。后续修复与最终实测结果见下文。

机器可读记录见 [原生续验证据](menubar-native-2026-11-14.json)。该记录保留失败尝试及最终通过证据，多屏单独标为用户要求暂缓。

### Chrome 全屏失败后的修正与复测

失败版本为 `2dbfe6f`。首先尝试在 `CanJoinAllSpaces` 与 `FullScreenAuxiliary` 之外补充 `CanJoinAllApplications`（提交 `5f3ec0a`）。[Apple 对该行为的说明](https://developer.apple.com/documentation/appkit/nswindow/collectionbehavior-swift.struct/canjoinallapplications)覆盖跨应用全屏空间；此 API 从 macOS 13 提供，低于 Orbit 的 macOS 15 最低要求。

该标记实际生效（collection behavior 为 262473），但用户复测仍反馈“仍无反应或跳到了其他桌面”。新增日志显示面板展开后 `on_active_space=false`，证明仅调整 collection behavior 不足以解决普通 NSWindow 留在其他桌面的问题。

最终改用 `tauri-nspanel 2.1.1` 将窗口转换为原生 NSPanel，增加 `NonactivatingPanel` 样式。展开使用面板的 `show_and_make_key`，去掉会激活整个 Orbit 应用的 Tauri `set_focus`。保留原 Tauri delegate，继续处理失焦事件。普通面板展开与 `Esc` 收起重新操作通过。

NSPanel 的首次全屏复测已能原地展开，但用户反馈点击 Chrome 空白处不会收起；对应日志未收到失焦事件。随后补充仅在面板展开期间注册的外部鼠标按下监听，在收起和退出时移除。它不拦截其他应用的点击，原有失焦关闭逻辑继续保留。[Apple 的事件监听说明](https://developer.apple.com/library/archive/documentation/Cocoa/Conceptual/EventOverview/MonitoringEvents/MonitoringEvents.html)区分了应用内与应用外事件，说明回调在主线程执行及监听结束后的移除要求。

最终真实单屏验收通过：

- 用户在 Chrome 原生全屏空白页点击真实 Orbit 圆环，确认面板原地展开，没有跳到其他桌面。
- 对应 `panel-settled` 记录显示主窗口隐藏且 `on_active_space=false`，面板 `on_active_space=true`，occlusion state 包含可见位。
- 用户点击 Chrome 空白处后确认自动收起；记录包含 `panel-outside-mouse-down`，随后约 3 ms 内 `panel-hidden` 且 `visible=false`。
- 同一轮实际图标点击再次展开，再点图标收起，也记录到对应原生点击与隐藏事件。
- 点击面板外的 Orbit 测试主窗口仍触发原生失焦并收起，没有破坏原有行为。
- 最终生产路径的 5 项桌面库测试与开启原生验收特性的构建通过。前端没有进一步变更，因此未重复运行此前已通过的前端全量检查。

单屏真实点击、失焦关闭及 Chrome 跨应用全屏验收已完成；多屏按用户要求保留待接屏补测。测试实例与临时标签页已关闭，没有安装、发布或更新已安装的 Orbit。
