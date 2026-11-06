# Codex 模型目录与接入

Orbit 根据当前 Orivelle 账户、节点和模型权限生成 Codex 用户模型目录，并将目录与用户配置作为一组文件管理。客户端的保存资格、确认与恢复流程见[桌面配置](desktop-configuration.md)。

## 固定版本与来源

当前原生保存目标是 macOS Apple Silicon 上的 Codex Desktop `26.1013.40934`，捆绑 CLI `0.163.0-alpha.10.2`，目录版本标识为 `codex-cli-0.163.0-alpha.10.2`。Orbit 核验已安装应用和 CLI 的签名、固定资源摘要与用户配置根。Windows 自动保存与恢复仍待适配。

编译器使用官方标签 `rust-v0.163.0-alpha.10.2` 对应提交的完整原始模型目录，保留工具、上下文、指令和嵌套元数据。升级时为新版本建立独立来源记录，核对模板与客户端合同，并更新编译及配置测试。

启用应用打包时，`licenses/codex/0.163.0-alpha.10.2/` 资源路径保留三份第三方资料。来源清单记录原始目录、版本与 SHA-256。[Apache-2.0 许可证](../crates/orbit-core/resources/codex/0.163.0-alpha.10.2/LICENSE)、[NOTICE](../crates/orbit-core/resources/codex/0.163.0-alpha.10.2/NOTICE)、[来源清单](../crates/orbit-core/resources/codex/0.163.0-alpha.10.2/PROVENANCE.json)

## Panel 绑定与目录生成

Panel 在获准模型投影中提供可选的 `codex_catalog_binding`。管理员为支持公开 `openai.responses` 的文本模型显式指定 `profile_id` 和 `template_slug`，模板限于固定目录的八个可见主模型。省略绑定字段保留原值，显式 `null` 清除。账户出口将合法绑定标为 `declared`，将损坏或已不合格的绑定标为 `unavailable`，两者均为 `verified: false`。无绑定的模型仍可按原有权限用于其他功能。

Orbit 读取当前账户和节点的模型，在所选供应商范围内按 Panel 顺序保留兼容的显式主模板绑定。有效推理档位取模板、Panel 支持集合及成员限制的交集，保留模板的顺序和完整选项。目录要求 Panel 提供有效的默认档位，并保留模板要求的多代理档位。所选模型必须符合这些条件，才能作为默认模型写入 `model`。超出条目或输出限制时拒绝整组。

目录编译器保留模板的未知字段及嵌套结构，替换 `slug`，清空 `upgrade`，并应用明确的推理档位限制。辅助项须是显式授权、从主模型可达的无环依赖，并在目录中隐藏。保留的 `auto_review_model_override` 要求同名、同模板的辅助绑定。当前 Panel 作用域规划只生成主模型，辅助能力需另行核实。[目录编译源码](../crates/orbit-core/src/codex_catalog.rs)、[作用域规划源码](../crates/orbit-core/src/codex_binding.rs)

## 用户配置写入与恢复

在接入页选择 Codex 模型并生成预览。预览展示目录来源、路径、摘要、别名和默认推理档位。应用前刷新账户、节点和模型信息，重新检查写入资格，并在账户锁内重新编译。配置目标、组织策略、本地写入资格、凭据或同作用域模型变动后，需要重新预览。

确认后，Orbit 通过事务日志管理两份文件，先写入 `~/.codex/orivelle-orbit.models.json`，再更新同目录的 `config.toml`。TOML 写入独立的 `orivelle_orbit` Responses provider、命令凭据 helper，以及 `model`、`model_provider`、`model_catalog_json` 和 `model_reasoning_effort`。存在 `profile` 选择或 profile 中覆盖这些字段时，Orbit 停止自动写入。Codex CLI 和 IDE 可能共用该用户配置，保存后需完整重启目标应用。

配置版本同时绑定两份文件的存在状态与内容摘要。恢复时选择原账户和 Panel，核验原安装身份、配置目标和适用策略，并在检查整组文件后还原原始字节。发现外部写入冲突时停止。退出后仍可在原安装身份有效时恢复文件，helper 授权继续受当前凭据状态约束。

## 运行来源与验证

已安装目标收据用于核验写入路径和版本。实际运行来源还取决于 Desktop 选择的 CLI 或 daemon、启动环境、有效配置根与进程连接。运行来源的核验范围是本机 backend，前台选中的远端 host 属于另一作用域。

运行来源判定只在 macOS 测试中编译，使用合成启动参数、环境、签名、进程身份、配置根和 stdio 连接。[运行来源判定源码](../crates/orbit-core/src/codex_runtime.rs)

生产运行来源保持 `unverified`。保存完成后状态为待重启，实际载入、模型选择与连接须分别取得客户端证据。当前验收范围见[验证指南](validation.md)。
