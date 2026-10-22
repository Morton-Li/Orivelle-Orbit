# 组织策略采集与配置门禁

2026-10-22 正在修正 Codex 的用户文件保存边界，见[桌面用户配置合同](desktop-configuration.md)。全局 `managed` 继续反映组织策略观测，远端未取得时仍为 `unknown`；Codex 另从完整本地来源推导用户配置写入资格。任何已知管理限制、不可读或未知本地来源均阻断。远端未知不被改成 `no`，也不生成载入或连接证据。以下首期观测记录描述此前要求全局 `no` 才保存的阶段；Claude 暂保留既有门禁。

2026-10-22 Claude 配置基础阶段已实现固定 `2.19675.0` 的独立安装目标收据、完整本地策略观测及界面资格校验，见 [Claude 合同](claude-configuration.md)。两个 MDM plist 全键探测加 182 个固定 schema/旧名的 forced 状态双次观测替代旧四键局部查询，不请求 CFPreferences 值；未知、缺覆盖、读取失败、非默认来源或版本不符仍阻断。完整收据每次重新验签，仅大 ASAR 哈希按文件身份复用；未实际运行本机策略观测。全局远端 `unknown` 保留，Claude 的 Service/事务保存入口本阶段仍关闭，诊断导出沿用旧门禁。下文较早阶段的局部采集描述是历史快照。

2026-10-15 的观测阶段在项目内实现策略来源观测、预览冻结和提交前复查，并用合成来源验证。当时没有运行真实客户端或系统策略查询；本地文件缺失不证明没有组织策略。后续真实系统集成结果分别记录在验证清单。

## 状态含义

`yes` 表示已观察到接管配置的管理来源；`unknown` 表示来源覆盖不全、权限不足、格式无法确认或身份/版本不匹配。只有完整且可信的观测才允许得出全局 `no`。Codex 用户文件保存采用独立的本地写入资格；Claude 诊断导出仍要求原策略资格。前端传入的摘要不能授予权限。

Codex 支持设备文件、macOS 管理偏好，以及云端和 Agent Security 交付的要求。本地来源没有内容不能排除后两者。本阶段不读取客户端登录资料，不启动另一个 CLI 来推断目标 Desktop 的策略；未取得远端覆盖证明时仍为 `unknown`。[OpenAI 管理配置](https://learn.chatgpt.com/docs/enterprise/managed-configuration)

Claude 也可能从 Enterprise Admin Console 或 bootstrap 接收配置。设备来源检查完成不等于运行实例没有远端配置；本地保存资格还需固定安装目标、受支持的本地选择和操作上下文，不能生成载入或连接证明。[Claude 配置交付](https://claude.com/docs/third-party/claude-desktop/installation)

## 设备来源

| 客户端与平台 | 本阶段的来源边界 |
| --- | --- |
| Codex / macOS | 固定系统 requirements、legacy managed 文件和官方偏好域的两个 TOML 键；不读取账户凭据 |
| Codex / Windows | 固定系统 requirements 与用户 managed 文件；实际 Windows 编译和查询仍未验证 |
| Claude / macOS | 分别观测官方 machine / per-user 两层 key；官方语义为逐键用户层优先，本阶段不重建或输出合并后的有效值；实际查询未运行 |
| Claude / Windows | 固定 HKLM / HKCU policy key；枚举直接 value 的名称、类型和修改时间，不读取 value data；实际查询未运行 |

Claude 的机器层存在直接的可见策略 value 时，可能屏蔽整个用户层，不能把两个 hive 按键合并，也不能只检查能成功解析的 value。子键和不受支持类型需要与直接 value 区分。仅影响更新、重启、复查和代理的官方例外键不等于接管推理配置；未知键、版本或来源覆盖不全仍不能得出全局 `no`。[Claude MDM](https://claude.com/docs/third-party/claude-desktop/mdm)、[读取规则](https://claude.com/docs/third-party/claude-desktop/configuration#how-keys-are-read)

原始策略值、文件正文和注册表内容不会进入前端、诊断日志或 SQLite。公开的组织策略状态使用 `managed` 和固定说明；Codex 另公开 `configuration_access` 及其摘要作为本地预览提示。来源材料和预览校验由 Rust 内部使用，UI 摘要不授予写权。隔离构建使用项目内假来源，不访问真实 CFPreferences、注册表或系统策略文件。

macOS 的 Codex 普通偏好值不等同于强制管理配置：非 forced 值保持 `unknown`；只有已确认强制的来源才能据此显示 `yes`。文件来源按已打开的普通文件描述符进行有界读取，拒绝符号链接和 FIFO，避免检查路径后被替换而阻塞。

## 预览与最后提交

策略观测绑定目标客户端身份、版本和来源。预览期间冻结观测结果；应用、恢复、诊断导出和启动恢复前重新采集，来源出现、消失、读取失败或摘要变化会使旧确认失效。最终检查覆盖 Vault、文件和 helper 绑定的提交入口，不能仅信任启动时缓存的 `ClientView`。

新操作组的策略摘要与备份密文通过 AAD v3 绑定。旧组保留 AAD v2 的读取兼容，缺少策略证明的旧 journal 不能自动继续发布；显式恢复需重新生成预览、冻结当前策略并确认。摘要不是策略正文，也不能充当客户端已经载入配置的证据。

Codex 界面收到本地写入资格受阻、未核验、摘要变化或已知 `managed=yes` 后立即作废旧确认；远端未知单独提示其可能影响生效。Claude 保留原策略门禁。原生层拒绝后，旧确认不能再次提交。固定中英文文案不展示原始策略正文，后台最终检查仍为权限判断依据。

这些检查不能锁住外部管理工具；最后读取与外部修改之间仍存在竞争窗口。多文件、Vault 与绑定保持可恢复协调，不声称全局原子事务。

## 验证与剩余工作

验证使用有界合成来源，覆盖来源优先级、出现/消失、格式/权限/大小异常、同一状态下的来源变化、过期预览及各提交/恢复出口的副作用限制。实际通过数量、失败日志、源码 SHA 和合并记录以阶段交付证据及[验证记录](validation.md)为准。

本阶段不会产生 `loaded_evidence` 或 `connection_evidence`，也不解决模型目录、Claude library envelope、真实 Desktop 联调或平台验收。Codex/Claude 的远端策略证明仍未取得；实际 CFPreferences、Windows registry 和专用系统集成测试尚未运行。
