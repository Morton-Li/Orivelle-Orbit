# 组织策略采集与配置门禁

本阶段在项目内实现策略来源观测、预览冻结和提交前复查，并用合成来源验证。不运行真实客户端或系统策略查询，不把本地文件缺失当作没有组织策略的证明。真实系统集成仍按原授权边界另行验收。

## 状态含义

`yes` 表示已观察到接管配置的管理来源；`unknown` 表示来源覆盖不全、权限不足、格式无法确认或身份/版本不匹配。只有完整且可信的观测才允许得出 `no`。预览、应用、恢复和 Claude 诊断导出都要求可允许的策略状态，前端传入的值不能授予此权限。

Codex 支持设备文件、macOS 管理偏好，以及云端和 Agent Security 交付的要求。本地来源没有内容不能排除后两者。本阶段不读取客户端登录资料，不启动另一个 CLI 来推断目标 Desktop 的策略；未取得远端覆盖证明时仍为 `unknown`。[OpenAI 管理配置](https://learn.chatgpt.com/docs/enterprise/managed-configuration)

Claude 也可能从 Enterprise Admin Console 或 bootstrap 接收配置。设备来源检查完成不等于运行实例没有远端配置，因此不能据此开放自动切换。[Claude 配置交付](https://claude.com/docs/third-party/claude-desktop/installation)

## 设备来源

| 客户端与平台 | 本阶段的来源边界 |
| --- | --- |
| Codex / macOS | 固定系统 requirements、legacy managed 文件和官方偏好域的两个 TOML 键；不读取账户凭据 |
| Codex / Windows | 固定系统 requirements 与用户 managed 文件；实际 Windows 编译和查询仍未验证 |
| Claude / macOS | 分别观测官方 machine / per-user 两层 key；官方语义为逐键用户层优先，本阶段不重建或输出合并后的有效值；实际查询未运行 |
| Claude / Windows | 固定 HKLM / HKCU policy key；枚举直接 value 的名称、类型和修改时间，不读取 value data；实际查询未运行 |

Claude 的机器层存在直接的可见策略 value 时，可能屏蔽整个用户层，不能把两个 hive 按键合并，也不能只检查能成功解析的 value。子键和不受支持类型需要与直接 value 区分。仅影响更新、重启、复查和代理的官方例外键不等于接管推理配置；未知键、版本或来源覆盖不全仍不能得出全局 `no`。[Claude MDM](https://claude.com/docs/third-party/claude-desktop/mdm)、[读取规则](https://claude.com/docs/third-party/claude-desktop/configuration#how-keys-are-read)

原始策略值、文件正文和注册表内容不会进入前端、诊断日志或 SQLite。公开状态只使用已有的 `managed` 和固定说明；来源摘要和预览校验材料由 Rust 内部使用。隔离构建使用项目内假来源，不访问真实 CFPreferences、注册表或系统策略文件。

macOS 的 Codex 普通偏好值不等同于强制管理配置：非 forced 值保持 `unknown`；只有已确认强制的来源才能据此显示 `yes`。文件来源按已打开的普通文件描述符进行有界读取，拒绝符号链接和 FIFO，避免检查路径后被替换而阻塞。

## 预览与最后提交

策略观测绑定目标客户端身份、版本和来源。预览期间冻结观测结果；应用、恢复、诊断导出和启动恢复前重新采集，来源出现、消失、读取失败或摘要变化会使旧确认失效。最终检查覆盖 Vault、文件和 helper 绑定的提交入口，不能仅信任启动时缓存的 `ClientView`。

新操作组的策略摘要与备份密文通过 AAD v3 绑定。旧组保留 AAD v2 的读取兼容，缺少策略证明的旧 journal 不能自动继续发布；显式恢复需重新生成预览、冻结当前策略并确认。摘要不是策略正文，也不能充当客户端已经载入配置的证据。

界面收到 `managed=yes/unknown` 后立即禁用旧确认；原生层拒绝后，旧确认不能再次提交。固定中英文文案说明需要重新检测和预览，不展示原始策略正文。后台最终检查仍为权限判断的依据。

这些检查不能锁住外部管理工具；最后读取与外部修改之间仍存在竞争窗口。多文件、Vault 与绑定保持可恢复协调，不声称全局原子事务。

## 验证与剩余工作

验证使用有界合成来源，覆盖来源优先级、出现/消失、格式/权限/大小异常、同一状态下的来源变化、过期预览及各提交/恢复出口的副作用限制。实际通过数量、失败日志、源码 SHA 和合并记录以阶段交付证据及[验证记录](validation.md)为准。

本阶段不会产生 `loaded_evidence` 或 `connection_evidence`，也不解决模型目录、Claude library envelope、真实 Desktop 联调或平台验收。Codex/Claude 的远端策略证明仍未取得；实际 CFPreferences、Windows registry 和专用系统集成测试尚未运行。
