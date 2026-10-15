# Desktop 协议隔离网关

此工具在项目内验证 Responses 与 Messages 的请求、流式事件、主/辅助模型和工具续轮，为后续真实 Desktop 联调准备可重复场景。它使用 Node 内置 HTTP 服务，仅监听 `127.0.0.1` 的临时端口，不转发、不调用模型、不执行工具或命令。协议测试不产生付费调用，也不授予 Orbit 登录或真实推理权限。

## 使用边界

`scripts/desktop-fake-gateway.mjs` 导出 `createDesktopFakeGateway`。实例创建时固定协议、场景和合成身份，返回回环地址、合成 token、只读摘要与可等待的幂等 `close()`。调用方负责在 `finally` 中关闭。token 只交给当前 Node 调用方，不写入控制台、测试报告、真实 helper、客户端配置或安全存储。

构造参数不接受上游地址、外部 token、任意响应模板或 headers。请求中的 query、header 和 body 也不能改写实例场景。模型从项目 fixture 固定选取；未知模型拒绝，不根据任意请求 ID 临时创建模型。

## 协议与场景

| 入口                             | 检查范围                                                         |
| -------------------------------- | ---------------------------------------------------------------- |
| `GET /v1/models`                 | 当前协议的固定合成主/辅助模型发现；不表示官方模型菜单元数据      |
| `POST /v1/responses`             | Responses 文本、单工具、并行工具及严格 call ID 续轮；JSON 与 SSE |
| `POST /v1/messages`              | Messages 文本、单工具、并行工具及 tool result 续轮；JSON 与 SSE  |
| `POST /v1/messages/count_tokens` | Messages 的合成计数合同；不作为真实 tokenizer 或收费证明         |

每个实例只启用一个协议。场景覆盖正常文本、单工具、并行工具，以及认证拒绝、429、5xx、流中错误、坏 SSE、提前断流和停滞。工具只是固定的合成结构；返回调用 ID 必须与本实例前一轮一致。错误、断线、取消、超时或关闭后不追加成功终态，不自动重发请求。

每个实例、每个模型角色同时只维护一条合成工具续轮；独立会话使用独立实例。辅助模型不能接续主模型的工具状态，其他实例的调用 ID 不能用于当前实例。

工具 fixture 固定为 `orbit_fixture_read`，验证的是协议结构与调用关联。真实 Desktop 的内置工具名称和参数尚未适配；后续需按目标版本实际声明的工具选择受控项目文件用例，不能把此合成函数的成功当成客户端已执行工具。Messages 的结果消息须紧跟调用消息，`tool_result` 连续前置，之后可以附合法文本。

SSE 与工具结构依据 [OpenAI 流式合同](https://developers.openai.com/api/docs/guides/streaming-responses)、[工具调用](https://developers.openai.com/api/docs/guides/function-calling)及 [Claude 流式合同](https://platform.claude.com/docs/en/build-with-claude/streaming)，只覆盖当前明确列出的协议子集。没有实现完整 SDK 或代理，不据此承诺两个 Desktop 对所有扩展字段的兼容。

## 资源与观测

硬上限为每实例 64 个请求、4 个并发、单请求 2 MiB、单响应 128 KiB、单流 128 events；默认存活 2 分钟，最多 10 分钟。测试使用更短期限并显式关闭。达到预算、期限或关闭时停止接入，清理连接及定时器；这些限制不能由网络请求提高。

摘要只保留固定协议/场景、合成身份、请求序号、模型角色、状态与计数。它不保存请求/响应正文、提示词、工具参数、headers、token 或原始异常。成功与错误用例断言精确请求数，证明测试驱动没有隐式重试；不能据此推断真实 Desktop 的重试行为。

## 验证与后续

运行 `node --test tests/build/desktop-fake-gateway.test.mjs`，测试只访问自身创建的回环服务。Mac 与 Windows 验证 job 均调用此分组；本机通过不代表远端 CI 或 Windows 已运行。实际数量、源码和日志摘要以[验证记录](validation.md)及阶段交付证据为准。

HTTP 回环服务只提供协议证据，不修改生产 HTTPS 校验。Claude 的 [V2 bootstrap schema](https://claude.com/docs/third-party/claude-desktop/schemas/bootstrap-config-v2.schema.json)明确还存在 schema 之外的客户端 URL 和 origin 校验；不能仅据 `format: uri` 判断真实 Desktop 会接受此地址。后续实际联调需要已授权的专用环境，核对目标版本、交付路径与 TLS 条件；不能通过关闭校验、修改系统信任或本地代理规避。

本工具不证明配置已载入、连接已验证、模型已出现在菜单、主/辅助模型真实执行或非 Claude 上游能力等价。完整 Codex catalog、Claude library envelope、真实 Desktop/Vault 与平台验收继续单列。

后续实际联调必须分别收集证据：

| 验收项     | 项目内隔离证据                                 | 真实 Desktop 仍需证明                                       |
| ---------- | ---------------------------------------------- | ----------------------------------------------------------- |
| 身份与版本 | 固定合成客户端/模型和请求关联                  | 目标运行实例、配置来源及实际使用的内置/覆盖 CLI 版本        |
| 策略与配置 | 来源变更、预览失效、文件/journal 故障          | 组织策略完整覆盖、获准写入、重启后实际配置载入              |
| 模型和工具 | 主/辅助模型选择、单/并行工具、续轮 call ID     | 菜单和别名、实际模型路由、客户端工具结果回传                |
| 故障       | 精确请求计数、流错误/坏帧/EOF/取消后无成功尾帧 | Desktop 的重试、取消、错误提示与会话可继续性                |
| 系统集成   | 假 Vault 和项目内文件                          | 专用环境原生安全存储、文件权限、helper 与应用更新后的连续性 |

配置保存、客户端载入和连接验证保持三个独立状态。fake gateway 或 helper 被调用不能单独升级后两项状态。真实客户端设置、安全存储和系统变更继续遵守用户“系统集成另授权”的边界；本阶段不执行这些步骤。
