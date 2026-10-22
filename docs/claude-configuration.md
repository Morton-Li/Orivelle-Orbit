# Claude Desktop 本地配置合同

本阶段补齐 Claude 的整组保存与恢复。本文的文件合同来自官方文档及固定安装包的只读核查；它不证明配置已被真实客户端载入，也不证明 Code 请求兼容或连接成功。

## 固定版本与文件

首个自动写入目标限定为 macOS arm64 的 Claude Desktop `2.19675.0`，bundle ID 为 `com.anthropic.claudefordesktop`。其他版本、非默认数据目录和 Windows 目标在取得对应合同前保持未核验。

[官方配置说明](https://claude.com/docs/third-party/claude-desktop/configuration)规定本地配置库及当前配置索引；[安装说明](https://claude.com/docs/third-party/claude-desktop/installation)要求应用配置后重启。固定安装包进一步确认同一 `~/Library/Application Support/Claude-3p` 根下的三份文件：

| 顺序 | 相对路径                       | 本次负责的内容                                            |
| ---- | ------------------------------ | --------------------------------------------------------- |
| 1    | `configLibrary/<新 UUID>.json` | 新建 Orivelle profile，正文为扁平配置键对象               |
| 2    | `configLibrary/_meta.json`     | 保留既有 entries 和未知字段，追加新条目并选择其 appliedId |
| 3    | `claude_desktop_config.json`   | 仅将 deploymentMode 设为 3p，保留其他字段                 |

新 UUID 不能覆盖既有文件。官方 V2 导入格式的嵌套正文不等于本地 profile 格式，不能直接写入配置库。固定公共代码仅用来核对读取和写入行为，不复制第三方实现。

本次核对的公开安装材料为 Info.plist SHA-256 `f18af671edea570f867f3f59ddd0ac8f9341275cb61f6262f85afb986ce68c31`；ASAR 内 `.vite/build/index.chunk-BZdcw7TE.js` 的 7,056,339 字节正文 SHA-256 为 `0e794998a1ad175f651818913cba690de0724d92d7a946efc58ae0f08c7df226`。这些是已安装版本的静态合同证据，不是稳定公共 schema 的承诺。

## 本地选择与管理来源

metadata 中出现任何 hybridPointer 时均阻止自动切换，包括空值或无效值。有效指针优先于 appliedId；本阶段不删除它来绕过原组织选择。当前选中 profile 使用 bootstrap 来源时同样阻止切换，其他未选中 profile 原样保留。

设备策略的完整覆盖、安装材料、默认根目录及本地选择必须分别核对。既有四个 forced-key 的局部检查不足以证明没有管理配置。无法读取、未知管理键、来源变化或覆盖不完整均阻止写入。全局 managed 与本地写入资格分开：远端组织状态仍可为 unknown；它不提供加载或连接证据。

保留官方模式选择、原登录资料和其他应用设置。将 deploymentMode 改为 3p 是明确展示的切换效果，作用于整个 Claude Desktop，包括 Chat、Code 与 Cowork；不能承诺仅修改 Code 的路由。

## 凭据、确认与恢复

新 profile 使用 [官方 credential helper](https://claude.com/docs/third-party/claude-desktop/credential-helper)，不写明文 key。helper 参数仅包含客户端与本安装绑定标识，不能携带秘密。配置中明确使用 Panel 授予的节点和模型；保存不自动发起推理请求。

预览只读取并生成脱敏差异，不创建客户端目录，不启用 helper。用户确认后，受认证的操作上下文绑定账户、Panel origin、安装、连接和授权版本、凭据引用、节点、模型、目标、策略及精确文件组。每次实际写入及启用授权前复查；三份文件全部读回成功后才启用新 helper。

沿用加密 journal 和备份。中断后按整组协调；任何文件出现第三方状态时停止整组覆盖。恢复原字节与恢复授权分别判断，退出、撤销、过期或遗忘的旧授权不得被恢复动作复活。没有全局原子事务；不遵守锁的外部写入仍可能与最后检查、替换竞争。

保存或恢复成功只报告配置已保存、待重启。实际加载、模型选择、Code 主辅模型、流式、工具和错误处理仍需独立真实客户端验收。

## 本阶段最少检查

- 合成配置库：扁平 profile、三文件顺序、未知字段保留、原选择恢复、秘密脱敏、大小和路径限制。
- 资格与 journal：管理/远端选择拒绝、目标晚变、三文件中断和重新进入、任一第三方状态整组停写、旧上下文拒绝、恢复不复活授权。
- Service 与界面：预览、确认、保存、重开和整组恢复；确认资格变化后旧预览永久失效；保存与载入、连接证据始终分离。
- 受影响的原生测试和 Mac 编译、前端类型/构建及相关浏览器测试。既有 Panel 授权通过证据不因本次 Claude 配置变更重复执行。
