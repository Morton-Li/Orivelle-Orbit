# Codex 完整模型目录编译

`orbit_core::codex_catalog::compile` 将明确的模型映射编译为完整的 Codex 目录 JSON。它是后续配置适配的纯函数入口；当前没有接入 Service、Panel 或真实客户端配置流程。

## 固定来源

本阶段只接受 CLI `0.163.0-alpha.10.2`，固定官方标签 `rust-v0.163.0-alpha.10.2` 对应的提交 `4ebc485ca321243727539187169425fcf972f2c4`。完整原始目录为 472072 字节，SHA-256 为 `943ca7fe1d19ed019054158303f0dbbd80b3bef43b3aa425a71aa7cb3fb52c2b`，含 11 个条目、50 种原始字段。

原件、Apache-2.0 `LICENSE`、完整 `NOTICE` 和来源清单保存在 [`crates/orbit-core/resources/codex/0.163.0-alpha.10.2/`](../crates/orbit-core/resources/codex/0.163.0-alpha.10.2/PROVENANCE.json)。模板包含完整指令、工具、上下文、reasoning 和嵌套元数据；只列模型 ID 不能替代这些内容。原件不修改，新版本应增加独立来源固定、合同复核和测试，不能安装时动态追踪 latest。实际分发模板的后续接线还须同时保留许可证和 NOTICE。

2026-10-21 仅只读检查公共安装包：Desktop `26.1013.40934` 捆绑该 CLI，内嵌目录与官方原件只差末尾 LF。证据在项目忽略目录 `.cache/evidence/codex-catalog-readonly-provenance-20261021.json`。Desktop 可选择其他 CLI 或 daemon，因此安装包相符不能证明运行实例的来源、有效配置或载入状态。

[OpenAI 官方部署说明](https://learn.chatgpt.com/docs/enterprise/roll-out-a-gateway)要求桌面部署匹配捆绑 CLI 的目录，并为自定义别名提供完整元数据。使用本地 `model_catalog_json` 时会替换内置目录，修改后需要重启。此函数不写入该设置。

## 输入与生成规则

`CatalogPlan` 的版本、来源 hash、授权别名和辅助依赖均由调用方声明。编译器验证这些声明内部一致，但不会授予账户权限，也不会验证上游模型、实际路由、工具能力或组织策略。生产调用方仍需从当前账户和节点权限获得可信映射，不能按模型显示名猜测模板。

- `authorized_aliases` 是允许集合；`mappings` 是本次要生成的准确子集，允许不输出其他获准模型。空映射、重复或无效别名、未获准的输出、未知模板均拒绝。
- `default_alias` 必须映射为可见的主模型。它作为编译结果独立返回，供后续显式设置配置中的 `model`；目录 priority 保持原样，不能据此声称已改变客户端默认模型。
- 每个映射保留完整原始对象，包括未知字段与完整嵌套值。仅修改 `slug`、将 `upgrade` 设为 `null`、按明确限制收窄 reasoning，以及将辅助模型的 `visibility` 设为 `hide`。原来隐藏的模板不会被提升为可见。
- 辅助依赖必须显式指向已映射且获准的辅助条目，形成从主模型可达的无环图；缺失、重复、孤立或循环依赖拒绝。非空 `auto_review_model_override` 保留原引用，要求相同别名和模板的明确辅助映射，不能让引用落回其他目录。
- `allowed_efforts` 与模板选项取交集，保留官方顺序及每个完整选项。交集为空、默认 effort 不在交集内，或移除了模板的 `multi_agent_reasoning_effort`，均拒绝；移除原默认值时必须明确提供替代值。输入列表中的其他 effort 不会新增能力。

辅助图只覆盖调用方声明和已识别的元数据引用。固定版本的 Guardian 选择逻辑在没有 override、也没有首选辅助条目时会回落到父模型，不应据此自动添加 `codex-auto-review`。其他配置、功能开关、插件或运行路径需要调用方另行核实，编译成功不表示所有隐式辅助请求都已覆盖。[固定版本选择逻辑](https://github.com/openai/codex/blob/4ebc485ca321243727539187169425fcf972f2c4/codex-rs/ext/guardian-reviewer/src/model.rs)

上限为 256 个授权别名、64 个输出条目、每个名称 128 字节、合计 256 条依赖、每项 16 个 reasoning 选择和 1 MiB 输出。序列化写入时检查输出字节上限，失败不返回部分目录。生成结果包含目录字节、SHA-256、明确的默认别名和主/辅助别名列表；公开错误只返回固定错误码，不回显完整模板或输入内容。

## 当前验收边界

项目内测试使用合成映射，验证模板保留、权限子集、引用和 reasoning 一致性及资源上限。没有启动 Desktop、helper 或 Orbit GUI，没有读取用户认证资料、修改真实配置、调用系统安全存储或发送推理请求。

Panel 尚未提供生产目录绑定，配置文件和目录的联合恢复也未接线；组织远端策略仍为未知。此阶段不会解除自动配置门禁，不会生成 `loaded_evidence` 或 `connection_evidence`，也不代表实际模型菜单、流式或工具调用已经验收。现有 Mac 候选包仍绑定此前的源码，不能作为本模块的打包或运行证据。
