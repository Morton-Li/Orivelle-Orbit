# 验证入口与历史结果

当前产品操作见[用户说明](../../README.md)，架构与数据边界见[运行机制](../architecture/runtime.md)，日常命令和发布要求见[开发、构建与发布](../development/build-and-release.md)。本页集中维护原生验收和历史关键结论；旧记录仅描述当时验收，新增结果按日期单列，不自动延伸至当前发布包。

## 按变更选择验证

### 2026-11-17 Codex 模型目录与同步

基于 Orbit `39828b1`、Panel `c09f10df` 的 `feature/codex-model-sync`，通过真实 Panel HTTP 测试复现已知模型支持推理但 `default_effort` 为空的投影：八种模板中只有 gpt-6.1-sol 声明了上游默认值。旧 Orbit 自动选择会将其余模型排除，并可能误报缺少声明。修复保留 Panel 对上游默认行为的语义，改由 Orbit 明确配置权限交集内的默认值。

- Panel：26 项、467 个断言通过，覆盖八种模板、公开别名、自动/手动声明优先级、原账户/节点权限、未知值与私有路由不外泄；相关应用 PHPStan、Pint、Vue/文档格式和 CI profile 清单验证通过。新增回归已纳入核心 CI。
- Rust：完整核心曾通过 308 项、3 项原有外部验收 ignored；最后的源快照复核、维护锁与 Key 绑定补充后，最终关联 Codex 46 项全部通过。包含模型新增/移除、默认档位为空、别名保留、外部配置冲突、断网、已绑定 Key 与当前默认 Key 分离、半发布恢复及精确恢复最初配置。
- 前端相关 100 项通过；lint、格式、类型、生产构建及夹具排除通过。macOS 原生 Tauri 编译检查通过。
- 这是合成账户、隔离 SQLite 和项目内临时配置的验证；没有发送真实推理请求或修改实际桌面配置。本次未取得用户 Windows 会话的线上原始返回，不能确认当时是否同时缺少模板声明；未做 Windows 原生、MySQL、发布或生产部署验收。

### 2026-11-16 图像 API 工作台

在独立 `feature/image-api-workbench` worktree 接入高级参数、PNG 蒙版/标注、流式临时预览及
SVG/GIF 原文件保存、受控预览和剪贴板复制；Panel 对应 `feature/orbit-image-api` 仅新增只读
`image_api_parameters` 字段，保留旧订阅目录。共享 `images-api-catalog.json` 是静态合成合同，
不能证明真实上游兑现。默认选项与透明背景继续走订阅，实际高级调整才使用专用池和独立销售价；
错误字段只来自本次冻结请求，不展示上游私有文本或未调整的参数。

前端完整检查通过 184 项单元测试、类型、格式、ESLint、生产构建及夹具排除。
18 项相关浏览器用例通过，覆盖 Unicode 的订阅字节界限与 API 字符上限、显式默认值、素材替换、
格式互斥、流式中间图、按参数报错、SVG/GIF 展示与复制/另存入口、再次编辑限制及旧图片工作流。
包含 1280 像素亮色 SVG 和 460 像素暗色 GIF 预览。Chrome 在项目隔离临时目录中运行；
原沙箱启动失败及旧按钮选择器失败保留日志，修正后的最终相关回归全部通过。

完整锁定 Rust 原生工作流通过 303 项 core、2 项 helper 及本机 macOS arm64 Tauri 编译。
3 项外部 core 验收、独立真实 Keychain 和签名探针共 5 项保持 ignored，不计通过。
新增专项覆盖 SVG 脚本/事件/DTD/外部资源和 CSS 混淆、深度与离屏图层预算、嵌入位图、RGBA 透明度，
GIF 的帧、时序、循环、子帧处置、截断与数量限制，以及超过解码器默认 50 MB 但符合显式预算的大帧。
HTTP 传输证明 EOF 前交付、状态/媒体类型/体积限制和禁止重放；Service 集成使用合成 HTTP，
验证蒙版/标注编辑、完整批次原字节保存、快照、SVG/GIF 导出与复制、预览归属和删除缓存。

Panel 静态合同及 Orbit 账户接口通过 27 项、324 断言，PHPStan/Pint 通过，共享目录夹具字节一致。
获用户明确授权后，仅在独立 worktree 内补充固定 Rust 图像依赖及 Cargo.lock，并通过项目镜像补齐
已锁定的原生缓存；未修改系统软件或本机配置。早期缓存缺失和隔离 core 验证不再替代完整原生结果。
本轮未使用真实 Vault、生产账户、付费图片或 Windows 原生验收，也未推送、打包发布或部署。

### 2026-11-16 API Key 管理

独立功能 worktree 完成 Key 查看、管理、本地默认选择与按客户端冻结绑定。确认弹窗仅显示应用、账户、节点与密钥，接入页使用 API Key 页的唯一默认选择；取消 Claude 接入默认保留主设置。普通通知 4 秒、错误通知 6 秒自动关闭。旧自动 Key 改由 Panel 定时维护和新版接口的升级兼容逻辑清理，不等待本地绑定、任务或替代 Key；手动/外部 Key、账务与安装 tombstone 保留。

最终前端检查通过 180 项单元测试、TypeScript、ESLint、格式、生产构建和测试夹具排除。40 项相关浏览器用例中，首轮 36 项通过，4 项旧详情展示断言按新界面修正后定向复测通过。验证默认 Key 与接入后绑定一致、单一取消入口、基础确认字段、通知自动消失、十进制字符串、只读及 1280/800/460 像素布局。可交互预览使用合成 IPC；夹具的重复确认 ID 改为逐次生成，避免一次过期后影响后续预览。

锁定 Rust 1.97.1 的原生工作流通过 288 项 core、2 项 helper 与 Tauri 编译。3 项外部 core 验收、独立真实 Keychain 和 signer 探针共 5 项保持 ignored，不计通过。Panel 本轮 44 项相关 HTTP/授权/维护回归通过（669 断言）；隔离 MySQL 8.4 的 10 项双进程并发通过（144 断言），包含清理先提交时旧客户端不能重新发行自动 Key。PHPStan、Pint 和新授权页 lint 通过。实际清理只在合成数据库验证，未连接生产 Key；Windows 原生、真实 vault、客户端网络与付费图片连续性未在本轮验收。

### 2026-11-16 正式标签后的上线检查

检查范围为正式 `v1.0.6`（`5f2f4cbdea3f6370839c2050de237351ee97a28f`）到本地候选 `develop`（`21a99b9d94e07822dcdd649a9f3c5d4bd1c8a7e7`）及直接受影响的消费者。此轮未修改产品源码；补齐旧本地 Panel 验收仍依赖自动发行密钥的过时假设，按当前 `api-keys-v1` 明确授权后选择密钥。

本机 macOS arm64 的完整 `npm run check` 通过：180 项 Vitest、格式、ESLint、类型、生产构建和夹具排除；宿主构建脚本 85 项通过；`native-workflow check` 通过 288 项 core、2 项 helper 和 Tauri 编译。完整浏览器 53 项通过，使用合成 IPC，并保存 API Key 宽屏及 460 像素暗色截图。首轮 Chrome 在沙箱内启动即 `SIGABRT`，未进入断言；获准在沙箱外使用相同入口后通过，原失败日志保留。

[本地 Panel 验收](../../crates/orbit-core/src/service/tests/panel_acceptance.rs)现覆盖真实会话的列表、创建、显式领取、揭示、轮换、编辑、重置计数、撤销和删除，并继续检查 TLS 链与主机名、错误 CA/SAN、CSRF、PKCE、安装证明、模型权限、离线节点、登出撤销和重新授权。轮换需证明旧凭据引用被持久化停用，新默认选择需在重新授权后以新的凭据引用恢复。丢应答场景额外要求任务所属的本地 TLS 代理：仅对名称为 `Synthetic Orbit recovery` 的首次创建，在真实 Panel 已返回成功后断开下游；代理不重发请求。原生消费者必须进入待核对状态、阻止另一次管理操作，再用同一持久化请求恢复并证明没有重复创建。旧 Caddy 夹具本身不提供此故障注入，不能直接宣称已覆盖丢应答。

该单一 opt-in 在本机完整通过：真实 Rust Service → 本地 HTTPS 代理 → 实际 Panel → 独立 MySQL；无网络 mock，使用合成账户与内存 Vault，不经过 Edge 推理。前三轮失败如实保留：旧夹具的 `Arc::get_mut` 无法处理 Service 自持的 Weak 引用，改为独占解包并重建正确 Weak；TLS 夹具把 CA 当作服务端证书，换为同根签发的终端证书；模型预期误填数据库 ID，改为协议实际的 slug。第三轮已完成密钥场景后在模型断言失败；修正专用数据与输入后复验完整同一入口，通过轮换、丢应答恢复、模型权限及登出重连的全部断言。原断言、TLS 校验及权限保护均保留。

用户明确 Windows 更新链路已由其他任务在 GitHub Actions 验证，排除本轮补验。此前的源码指纹核对作为辅助证据保留，不将其他任务的 Actions 或[历史回执](../../tests/acceptance/desktop/windows-updater-2026-11-15.json)记为本轮执行。

本轮未使用真实系统 Vault、修改真实客户端配置或发起付费图片请求。相应外部验收继续显式 opt-in，未运行项不计通过。

| 变更范围                           | 入口与必须区分的边界                                                                                                                                                                                       |
| ---------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Vue、状态和 typed IPC              | `npm run check`；页面交互另用 `npm run test:e2e`。浏览器测试使用合成 bridge，不能替代 Tauri、系统凭据或实际平台调用                                                                                        |
| Rust core/helper、持久化及配置事务 | [原生工作流](../../scripts/native-workflow.mjs)与相关 Rust 测试；核对预览失效、文件变更冲突、应用/恢复、跨账户迟到响应和任务幂等                                                                           |
| 账户、用量与平台接口               | [Panel 验收夹具](../../tests/acceptance/panel/)及 [Rust 消费者](../../crates/orbit-core/src/service/tests/panel_acceptance.rs)。独立数据库、合成账户和进程内 CA 不代表系统 Vault、现有业务库或真实用户钱包 |
| 安装、系统交互和旧版升级           | 在目标 OS 使用真实包与系统控件，保存源码/可执行文件/安装包摘要、旧数据、窗口/托盘、退出保护及安装前后回执；不能用交叉编译或浏览器模拟替代                                                                  |
| 发布和更新                         | 分别核对双平台 package evidence、DMG/NSIS 和 updater 签名；包可创建、签名可校验、实际安装/重启成功是不同结论                                                                                               |

运行前固定源码、依赖锁和数据目录，明确哪些 peer、HTTP、Vault 和资金端采用合成实现。生产构建必须排除 demo/fixture；演示应用的独立标识、数据库和非持久化 WebView 不能证明正式应用的数据迁移或登录链路。

外部验收保持显式 opt-in，具体保护以测试源码为准：

- [真实图片入口](../../crates/orbit-core/src/service/tests/image_live_demo.rs)：需要当前明确的图片预算及专属 Docker fixture。
- [真实本地 Panel](../../crates/orbit-core/src/service/tests/panel_acceptance.rs)：需要受保护的独立 Panel/MySQL/TLS 与私有输入。
- [真实 Codex 配置与 Keychain roundtrip](../../crates/orbit-core/src/service/tests/macos_config_roundtrip.rs)：会接触默认配置与系统凭据，不能算进普通合成检查。
- [NativeVault](../../crates/orbit-core/tests/macos_native_preflight.rs)与[CLI signer](../../crates/orbit-core/tests/updater_cli.rs)另有独立 ignored 入口。

未运行或 ignored 项不计为通过；历史任务的预算、凭据及写入授权不延续。

## 历史桌面与安装包验收

| 日期与样本                             | 已执行的结果                                                                                                    | 适用边界与保留的失败                                                                                                               |
| -------------------------------------- | --------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------- |
| 2026-11-11，macOS arm64                | 隔离 `.app` 的托盘创建、关闭隐藏、单实例唤回、菜单路由与合成任务退出保护通过                                    | 未自动化直接鼠标点击菜单栏图标，未证明隐藏期间真实 Claude 流量连续；未安装更新或执行系统注销                                       |
| 2026-11-12，Windows v1.0.1→候选 v1.0.2 | 真实 NSIS 安装/升级、Windows 凭据及旧设置/图片历史保留、原生托盘/对话框/剪贴板/保存文件、退出取消和后台任务通过 | Panel、图片和用量是受控 HTTPS peer；非真实上游或 Codex/Claude 配置写入验收。最早 v1.0.2 发布编译因图标借用生命周期失败，修复后通过 |
| 2026-11-12，macOS DMG                  | 实际 verify/只读挂载、Applications 链接、背景精确摘要、Finder 位置和窗口布局通过                                | 仅为安装介质及布局验收，不是安装后在线升级、重启或卸载验收                                                                         |
| 2026-11-13，配置格式兼容               | 本机 Codex/Claude 只读 preflight 与合成配置/恢复回归通过；不再按精确版本或固定文件摘要限制接入                  | 版本仍使旧预览失效；未改真实配置、未发送真实模型请求，不代表未来配置格式或 Windows 自动配置均受支持                                |

Windows 的[原生回执](../../tests/acceptance/desktop/v1.0.2-windows.json)绑定验收源码 `99b2eb4`、应用构建 `3c64e7f`；其中旧参数复用只描述当时版本，当前入口已移除。NSIS 负载与安装文件只允许固定 Tauri 的 `__TAURI_BUNDLE_TYPE_VAR_UNK`→`__TAURI_BUNDLE_TYPE_VAR_NSS` 标记变化，其他字节须相同，不能泛化为任意哈希差异。DMG 的[布局回执](../../tests/acceptance/desktop/v1.0.2-dmg.json)绑定源码 `e1f379c`、构建 `29f4cec`；原生截图属于当时 Actions artifact，不保证永久可下载。

这些结果只覆盖表列候选的原生菜单与托盘行为；后续快捷面板、全屏空间或托盘实现变更需按实际影响范围重新验证。

## 2026-11-15 Windows 原生补齐

本次已使用 Windows x64 VM 构建、安装并验证实际 NSIS 候选。原生启动与托盘、Codex MSIX 的 helper 认证请求、Claude MSIX 的实际 GUI 请求、完整卸载后的完整 EXE 重装复验与配置恢复，见[当前验收范围](../../tests/acceptance/desktop/windows-tray.md)及[机器回执](../../tests/acceptance/desktop/windows-native-2026-11-15.json)。源码文件清单、包与可执行文件摘要分别绑定，不能用本节替代正式发布签名或推送证明。

客户端请求使用真实安装程序、Windows Credential Manager 和默认配置路径，远端为专用受控 HTTPS peer。Claude 首次运行会改写主设置，因此另验收明确选择保留当前设置的恢复模式；严格恢复仍拒绝冲突。单屏已有 WebView2 环境不证明真实多屏 DPI 或全新系统运行库引导安装通过。Windows 默认更新设置的变更没有改变该次未签名候选的手动安装包更新能力；后续正式渠道的应用内更新见下节。

普通用户补验另外定位并修复 WindowsApps 目录访问及导出文件多余的所有者权限请求；提升权限的运行结果不能替代普通用户业务验证。主窗口在 Windows/macOS 共用固定标题栏和内容区独立滚动，另验收内容边界、返回位置与客户端定位。历史样式包和后续权限修复的产物分别记录，当前安装包以机器回执的 `current_delivery` 为准。

## 2026-11-15 Windows 应用内更新

[Actions 实装验收](https://github.com/Morton-Li/Orivelle-Orbit/actions/runs/43730209076)在 GitHub-hosted Windows x64 上完成真实 NSIS `0.2.0`→`0.3.0` 更新与自动重启。启动检查更新、错误包/版本签名拒绝、下载取消、维护互斥、安装前缓存篡改检测均通过；设置页下载后自动弹出确认框，确认后在含空格的原安装目录更新，新程序与 helper 的摘要均与构建回执一致，语言/主题/启动检查选择及现有用户数据文件保留。[机器回执](../../tests/acceptance/desktop/windows-updater-2026-11-15.json)保留源码、包摘要、复用证明及先前失败。

安装包来自[构建运行](https://github.com/Morton-Li/Orivelle-Orbit/actions/runs/43727014430)的 `425bc43`，按测试版本派生构建提交；最终验收脚本为 `1bc6112`，仅修改确认对话框断言。同仓库祖先关系、应用输入未变、安装包摘要及两个版本签名验证通过后复用同一批包，未声称重新编译。构建运行中的前端 168 项、Windows core 227 项（另 2 项 ignored）、helper 2 项和原生桌面编译检查通过；本机另通过 7 项更新核心测试与 macOS 桌面编译检查，使用已有 Rust 1.98.0，项目锁定工具链仍为 1.97.1。

此次使用临时 updater 密钥、受控 HTTPS peer 和已有 WebView2，不使用正式发布私钥，不创建 Release。安装包写入与重启使用真实系统流程；这份更新验收只覆盖设置和既有数据文件保留，真实客户端请求及凭据沿用上节独立回执。Windows 采用 NSIS 的安装及错误处理，未验证 macOS 式数据库/应用包自动回滚。普通候选保持空频道；旧版 Windows 需要先手动安装一次支持更新的新正式版本。

## 历史图片链路

2026-11-11，在当时最多三张图片预算内实际发送两次：一张生成、一张以生成结果为参考的编辑，均 `n=1`，没有自动重试或使用第三张额度。候选为 Orbit `5f8ae5d`、Panel `c420b919`、Edge `8b74d79c`。

真实请求由 Orbit 原生 `Service::send_image_once` 经 Docker 内实际 Panel 授权/结算和正式 Edge 执行代码到上游。原生入口使用合成会话、FakeVault、受绑定推理 key 和独立 SQLite 钱包；进程内 CA 不修改系统信任或 hosts。上游 access token 由短期内存 broker 提供，不落入业务数据库、不执行 OAuth 刷新。此结果没有覆盖本次桌面登录、真实系统 Vault、真实用户钱包、MySQL 并发或正式构建身份。

| 事实               | 生成                                                               | 编辑                                                               |
| ------------------ | ------------------------------------------------------------------ | ------------------------------------------------------------------ |
| 请求               | `1024x1024`、low、opaque、PNG                                      | `1536x1024`、low、opaque、JPEG/65                                  |
| 完整解码与原始保存 | `1536×1024` PNG                                                    | `1536×1024` PNG                                                    |
| 原始字节 SHA-256   | `e3d53041ca008c9df047e30af72ed3155cf8a71b6b1e232d7375026e40a8a132` | `39d4ea4b8b160f9ddf5f7c0fbc3cddf14540e6b24d32270125bc208c29b0c7d9` |
| 本地合成销售金额   | USD 0.01                                                           | USD 0.01                                                           |

每次均只有一个 attempt、唯一终态 event 和资源 permit，沿 `image_count` 合成结算并释放租约。未验收 token 销售计量、真实采购成本或剩余额度。上游没有提供实际执行模型身份；成功事件可选 HTTP 状态为空，不能写成已观测 200。尺寸/格式差异说明参数可提交不等于上游兑现，不能从两张扩展为全部模型、透明背景或参数组合通过。

首次原生准备遇到独占性检查失败，首次结算断言误把可选状态当必填，均未增加上游请求。收尾移除了内存 broker、撤销合成 key 并停用 demo 模型；旧本机预览地址不作为现行服务入口。原证据安全摘要位于当时 worktree 的 `.cache/live-demo/evidence/acceptance-summary.json`，不保证当前磁盘仍有副本。

真实图片入口默认 ignored，要求 `ORBIT_IMAGE_LIVE_DEMO=1`、绝对 `ORBIT_IMAGE_DEMO_ROOT` 和 `ORBIT_IMAGE_DEMO_OPERATION=generation|edit`，fixture 内含合成输入、进程内 CA 和证据目录。每个操作在发送前用 `create_new` 预留证据文件；文件已存在时拒绝重放，编辑只读前次已验证文件。重新准备目录不产生新的付款授权。

## 历史用量、工作台与发布复查

2026-11-13 的用量功能候选 Orbit `1611b791` 与 Panel `47db1d83` 完成本机合成 UI、Rust、Panel HTTP 回归；验证本人全部入口、多币种十进制金额、未知费用、时区、只读账户、跨账户迟到响应、筛选/分页/详情。后续错误提示布局检查覆盖 1280/800/460 像素及失败重试。没有真实平台联调或 MySQL 验收，平台汇总仍依赖提供 `/api/v1/orbit/usage/summary` 的 Panel。

同日图片工作台回归验证模型显示名在任务及快照持久化、旧 JSON 缺字段兼容、模型移出目录后的读取、统一缩略图框和原比例放大。请求仍用模型 ID；移除的参数复用不复活，历史图片仍可用于再次编辑。SQLite schema 未变，原请求参数/返回/结算数据继续供保存和恢复使用。这些是合成数据与本机 demo，不产生真实图片消费；当轮 Windows 只做脚本语法检查。

上线前复查记录绑定稳定基线 `v1.0.3`（`33a5e56dffe0a27c86c6b3e80a6cc554a1b96f59`）、候选 `d419fe4c21bfde94324ce5dca06339a775dbc209`，归档提交 `1bf3d47`；不是今天最新发布或候选状态。原记录的 npm check、Playwright、构建脚本、core/helper 和 typed Tauri 检查通过；唯一新门禁问题是工作台报告的 Prettier，修复于 `5978798`。网络/安装/更新等未变链路沿前轮精确候选证据复用，没有声称重跑。

首轮 Chrome 因沙箱 EPERM/SIGABRT 未进入页面断言，构建脚本首轮因临时目录缺失报 ENOENT；环境具备后同入口通过，原失败不改成首轮全绿。core 三项外部验收及独立 Keychain、CLI signer 仍 ignored，未计通过。未执行真实上游/消费、正式原生网络全链路、该候选 Windows 原生、正式签名 `.app` 实机在线更新/安装/重启/卸载或远端 CI；旧 Windows/DMG 回执不替代这些后续候选验证。

## 历史追溯与新增证据

原阶段文档已归并，本地 `63631f2` 保留整理前版本，可用 `git show 63631f2:<原路径>` 查阅：

- `docs/testing/release-gate.md`：两轮发布复查的候选、逐项结果及证据工作区。
- `tests/acceptance/desktop/README.md`：菜单/托盘、Windows 与 DMG、配置兼容的原记录。
- `tests/acceptance/images/README.md`、`tests/acceptance/images/workbench.md`：两张真实图片与后续工作台合成验证。
- `tests/acceptance/usage/README.md`：用量及错误布局的分段结果。

新增报告写清完整源码、平台/依赖、实际包和 peer、执行命令、失败/跳过及完成条件；只在当前验证主题追加仍有维护用途的结论。原始日志、截图与私有输入放入明确归属的证据目录，不能继续逐轮复制进 docs 或当作自动化 fixture。
