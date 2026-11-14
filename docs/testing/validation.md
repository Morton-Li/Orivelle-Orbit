# 验证入口与历史结果

当前产品操作见[用户说明](../../README.md)，架构与数据边界见[运行机制](../architecture/runtime.md)，日常命令和发布要求见[开发、构建与发布](../development/build-and-release.md)。本页集中维护原生验收和历史关键结论；以下结果来自原报告及保留的回执，本次文档整理没有重跑业务测试、访问远端 CI、安装应用或调用真实上游。

## 按变更选择验证

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
