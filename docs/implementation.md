# 实施与安全合同

## 分层与页面

Vue 3、TypeScript、Vite、Pinia、Vue Router、Tailwind CSS、Reka UI 负责界面和交互。Rust 负责远端请求、文件、凭据、配置、图片任务和 SQLite。前端 IPC 仅开放具体领域操作，定义在 `src/shared/contracts.ts`；统一使用 `{ request: ... }`。

首页优先呈现两个桌面客户端接入行，账户和余额保持紧凑。接入页分别选择获准节点和两个客户端模型，预览精确路径、版本、账户、节点、影响和脱敏差异。图片历史在工作台页内。设置包含外观、语言、磁盘、凭据、备份、关闭行为、更新和诊断。窄窗口按容器重排，导航支持 Control/Command + 1–4；弹窗管理焦点并恢复到触发控件。

界面不读取 API key，不提供任意 HTTP、SQL、文件或命令执行入口，不使用 localStorage 保存账户或凭据。渲染任务、图片和预算使用当前账户/Panel origin；模型异步请求绑定连接、节点和凭据 revision，旧请求不得更新新上下文。整个状态采用递增 revision，拒绝旧状态及晚到的命令确认覆盖更新事件。

## 登录、凭据与恢复

Panel 复用现有网页登录、验证、2FA/passkey 与权限服务。独立桌面授权使用系统浏览器、短时单次授权码、state、PKCE、HTTPS 交换和绑定的 loopback callback。回调只携带短时代码、state、issuer，不能携带账户或推理凭据。

Panel 地址固定为 `https://orivelle.powerbyte.io`。登录界面只显示官方服务；`start_login` IPC 只接收空对象，拒绝附加 origin 字段，原生层使用编译期常量。原生 Panel 请求和会话读取在凭据访问前拒绝其他 origin，历史连接也不能继续访问其他站点；没有环境变量、配置文件或普通 debug 覆盖入口。测试站点仅由 `cfg(test)` 的显式 Http fixture 放行。此约束不限制 Panel 授权返回的 Edge 推理节点，也不改变原账户/origin 身份的历史备份离线恢复。

安装 Ed25519 身份、备份加密密钥、桌面账户会话和推理 key 分别存入原生系统安全存储；安装证明不能代替用户授权。前端和 SQLite 只取得不含秘密的元信息和引用。不可用、拒绝访问或缺少凭据明确失败，不回落明文。refresh 单次旋转；响应丢失或本地写入失败需重新授权，不重试旧 refresh。

浏览器授权 pending 使用请求 ID 和期限独立表示；连接、拒绝、过期、失败、取消均返回关联终态。取消会关闭本地请求并阻止迟到回调写入凭据。正常会话轮换保留同账户作用域和草稿，同时暂停新的付费操作；轮换失败进入过期状态。首次授权已成功但账户查询失败时，可按 active connection 和 Panel origin 重新发只读查询，取得账户前不能执行付费操作。

账户和消费的手动刷新仅查询 Panel，冻结连接 revision、账户、origin 和会话 epoch。消费响应只合并服务端事实，不覆盖本地保存、取消或图片删除；首页保留最后一次成功查询的时间，失败不能把旧缓存冒充最新数据。

安装关联推理 key 由现有 Panel 发/取服务在业务权限和锁内创建或复用。只读成员仍可查看账户、预算和消费，不能自动取得接入 key。撤销冻结预览时 key revision，服务端在关联/key 锁内匹配，避免撤销后来替换的 key。

退出清除对应账户与推理凭据、禁用 helper 授权，保留安装和备份密钥。删除连接与撤销服务器推理 key 是独立选择。忘记安装须预览 Panel origin、涉及账户和备份，并确认旧备份恢复能力会丢失。目标客户端可能缓存凭据，不能宣称退出 Orbit 立即使它退出登录。

配置备份按完整操作组加密，每个客户端最多 5 组、保留 30 天。恢复使用原备份的账户/origin/安装身份、目标版本、组织策略和当前文件 hash；不依赖当前推理 key。用户明确选择原账户与 Panel。检测第三方修改后停止覆盖；文件、vault 和 helper 绑定采用可恢复 journal，不能声称全局原子事务，也不能声称 hash 阻止所有外部写入竞争。

组织策略由 Rust 观测固定设备来源，绑定客户端身份和版本，并在预览、应用、恢复、诊断导出及启动恢复时重验。仅在来源覆盖完整且允许切换时继续；两个客户端目前都缺远端策略覆盖证明，本地来源缺失仍为 `unknown`。新操作组以 AAD v3 认证策略摘要，旧 AAD v2 备份保留显式恢复兼容，不能自动继续发布。界面旧确认一旦因策略或上下文失效，必须重新生成预览。实现及未验证来源见[组织策略采集](managed-policy.md)。

启动恢复只继续已经明确确认的发布组；prepared 预览不自动发布。恢复与备份保留使用同一目标锁，备份指针和源 journal 终态在同一 SQLite 事务中提交。冲突会阻止新配置预览，并允许原账户检查恢复预览；文件仍不满足原组 hash 时不能覆盖。凭据删除失败保留不可用状态和秘密清理引用，不能重新授予 helper 权限。

## 桌面官方合同

Codex 桌面使用用户 `config.toml` 的自定义 provider 和 command credential helper。macOS 为 `~/.codex/config.toml`，Windows 为 `%USERPROFILE%\.codex\config.toml`。用户配置可能影响 CLI/IDE，保存后需完整重启。只修改负责字段与独立 provider，不改 `auth.json` 或 OAuth 身份。[官方接入](https://learn.chatgpt.com/docs/enterprise/connect-to-a-gateway)。

Claude Desktop 使用官方第三方推理配置和 helper。首个自动配置目标限定为 macOS arm64 `2.19675.0` 的默认 `~/Library/Application Support/Claude-3p/`：创建新的扁平 `configLibrary/<UUID>.json`、更新 `_meta.json` 的选择，并在同一根目录切换 `claude_desktop_config.json` 的模式；V2 导入正文不能直接当作本地 profile。Service 已接通预览、确认、整组保存及逆序恢复，具体资格和边界见[本地配置合同](claude-configuration.md)；其他版本、非默认目录和 Windows 目标仍待对应核验。第三方模式作用于整个 Desktop，首期以 Code 为验证重点，保留组织策略和原登录资料。[配置](https://claude.com/docs/third-party/claude-desktop/configuration)、[凭据 helper](https://claude.com/docs/third-party/claude-desktop/credential-helper)。

当前权限能力使用 `openai.responses`、`anthropic.messages`、`openai.images.generations`、`openai.images.edits`。未经实际客户端观测，不承诺非 Claude 上游与 Claude Code 的能力等价、任意别名自动出现、Fast/tier/reasoning 的速度提升。保存配置不自动执行收费推理。

## 图片与账务

只发送一次严格 JSON POST 到生成或编辑接口；完整请求包含 base64 和 JSON，限制为节点准入上限与 64 MiB 中的较小值。提示词最多 32768 UTF-8 字节，单图最多 32 MiB，响应最多 64 MiB。真实 PNG/JPEG/WebP 类型、尺寸、像素、解码、并发、预览缓存与磁盘均有界。本地图片独立存储，SQLite 不保存大量 base64。

任务冻结账户、origin、连接/凭据 revision、节点、模型 revision、参数和输入快照。已发送任务的取消、断线、超时或退出无法证明服务端停止，保留结果待确认。恢复只处理已收到的本地材料和未完成保存，不请求服务器图片结果，也不重放生成。逐图删除仅影响所选本地图片；多图任务的其他结果和服务器账务不受影响。

复制和另存为在排队或打开保存对话前冻结账户、origin、安装、连接 revision、授权 epoch 和所选图片材料。读取后校验接收时保存的 hash、真实类型和尺寸；最终在账户锁内再次核验作用域、有效期、忘记安装、维护状态及材料元数据，然后才写剪贴板或创建目标临时文件。退出、切连接、重新授权、读取前的合法文件替换或提交前的材料元数据变化会使旧请求失败；已经校验的原图片字节可继续用于输出，不会因之后的路径替换而改变。此处有隔离假剪贴板和项目文件证据，真实系统交互尚未验收。

记录 `X-Orivelle-Request-ID`，经 actor 作用域查 Panel 的统一执行和消费投影。服务端 `result_state`、nullable `outcome`、`charge_state` 和费用分项各自保留，图片到达与 HTTP 200 不能证明结算。金额采用准确十进制字符串；未知、待核账、冲突、缺失和真实零保持不同，不合并混合币种。

价格区分平台销售价、官方参考、平台配置参考价、运营估价和未知，保持 meter/unit_size。配置参考价不能冒充官方参考价；部分价格缺失仍显示未知，真实零价单独保留。缓存输入是输入子集，不重复累计。旧 `image_count` 明示按张计费；结果张数不能冒充 token。准入预算可能是软约束，不承诺在途并发永不超额。

## 更新、签名与参考

Aplena 参考固定于 `1359d012f3af2e45fb182b16b8c51bbb36bd2e07`。它提供应用签名与独立 updater 签名模式，没有已实现的安装登录身份/vault 方案；不把发布证书当安装用户身份，也不复制私人密钥。其 Morton Li 自签名 macOS 证书不是 Developer ID 或 notarization 证明。[Aplena](https://github.com/Morton-Li/Aplena)。

Updater 固定候选 2.13.3，CLI 2.12.2。更新下载必须 HTTPS、有界，校验跳转、平台、架构、固定可信公钥和受全局签名保护的版本；解包/安装消费同一份已验证字节或封闭 handle。安装前检查任务/helper/journal、数据一致备份和 schema 兼容，失败保留可恢复旧应用和数据。没有正式可信渠道和公钥时保持 disabled，不访问占位地址，也不把测试信任密钥编入正式候选。

GUI 在打开数据库或恢复任务之前取得独立进程锁，持有至退出；helper/更新协调使用另一把锁。macOS 可关闭窗口保留进程，并由应用重新打开窗口；Windows 当前拒绝保留隐藏进程的选项，旧设置也不会令窗口隐藏。

安装更新持有跨异步步骤的维护 lease，按实际 worker、待保存材料、pending 授权和在途修改检查空闲，不能只相信 UI 取消后的任务状态。维护期间暂停新的付费请求、凭据/配置/文件/元数据修改，保护 schema 备份和安装检查的一致性。失败释放 lease 并通知界面；若安全状态无法写回，则保持禁止修改直到重启。这里记录的是隔离锁与故障夹具证据，尚无真实安装或更新恢复验收。

CC Switch 仅作路径、格式和技术行为参考，实际检查版本为 4.0.6、MIT、无 Git 元数据。UI 独立设计。当前没有搬入 CC Switch 或 Aplena 的实质代码；后续实质复用须记录必要性、维护边界和版权许可。
