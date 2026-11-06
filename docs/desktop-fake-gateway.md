# 桌面协议隔离网关

网关供 Node 测试驱动在回环地址复现 Responses 和 Messages 的协议交互。每个实例使用固定合成模型、身份和工具数据，所有响应由项目内夹具生成。

```sh
node --test tests/build/desktop-fake-gateway.test.mjs
```

[网关实现](../scripts/desktop-fake-gateway.mjs) · [模型与工具夹具](../tests/fixtures/desktop-gateway/models.json)

## 实例与场景

`createDesktopFakeGateway` 在 `127.0.0.1` 上分配临时端口。创建时选择 `responses` 或 `messages` 和一个场景。调用方使用 `baseUrl` 与仅供本次测试的 `fixtureToken` 请求，完成后在 `finally` 中等待 `close()`。每个实例只有一套协议与场景，独立会话使用各自的实例。

| 入口 | 检查内容 |
| --- | --- |
| `GET /v1/models` | 所选协议的合成主模型和辅助模型。 |
| `POST /v1/responses` | 文本、单工具、并行工具及匹配 call ID 的续轮，支持 JSON 和 SSE。 |
| `POST /v1/messages` | 文本、单工具、并行工具及匹配 `tool_result` 的续轮，支持 JSON 和 SSE。 |
| `POST /v1/messages/count_tokens` | Messages 的合成计数响应。 |

工具名固定为 `orbit_fixture_read`，仅验证调用结构和续轮关联。每个实例的每个模型角色最多保留一条续轮状态。Messages 的结果消息必须紧跟工具调用，连续的 `tool_result` 位于可选文本之前。故障场景覆盖认证拒绝、429、5xx、流中错误、坏 SSE、提前断流及停滞。

## 资源与验收范围

实例上限为 64 个请求、4 个并发、单请求 2 MiB、单响应 128 KiB、单流 128 个事件。实例默认存活 2 分钟，最长 10 分钟。测试可调低预算，关闭或到期会清理连接、计时器和工具状态。

`snapshot()` 返回脱敏副本，包含场景、合成标识、请求序号、模型角色、状态与计数。提示词、请求正文、工具参数、headers 和 token 留在当次交互。用例对请求次数及错误后的流状态作断言。

回环 HTTP 夹具的证据范围限于合成协议。真实 Desktop 的配置载入、模型菜单、请求路由、工具执行、重试和 TLS 行为需在对应客户端版本与受控环境中另行验证。[验证说明](validation.md)
