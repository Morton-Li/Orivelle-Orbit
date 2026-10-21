# Codex 完整模型目录编译

`orbit_core::codex_catalog::compile` 将明确的模型映射编译为完整的 Codex 目录 JSON。它是纯函数；账户授权、配置写入和真实客户端载入分别需要自己的合同与证据。

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

## Panel 显式绑定

Panel 的可选 `provider_models.codex_catalog_binding` 只保存 `profile_id` 和 `template_slug`。固定注册表将 `codex-cli-0.163.0-alpha.10.2` 展开为上述 CLI 版本与原始目录摘要，模板只开放八个原本可见的主模型。它不复制完整模板，不按供应商型号或显示名推断映射，也不给辅助模型增加授权。

绑定沿用现有模型编辑权限、行锁和审计事务；仅允许公共供应商的 text 模型、公开调用范围、明确的 `openai.responses` 能力与不超过 128 字节的有效公共别名。省略字段保留原值，显式 `null` 清除；绑定本身不启用模型、不改价格或授予权限。最终有效行不满足条件时拒绝绑定写入；失效旧值允许清除。

账户出口在原有账户、组、成员与模型过滤之后投影绑定：未配置为 `null`；合法声明为 `state: declared`、`verified: false`，附固定 profile、CLI、来源摘要和模板；未知、损坏或已不合格的值仅为 `state: unavailable`、`verified: false`，不回显存储原文。声明不是运行实例或上游能力证明。reasoning 的支持集合、成员允许集合与默认值仍以原有策略为准；其 `revision` 是 `null` 或成员版本整数，不能按字符串摘要猜测转换。

该合同的 Panel 源码为 `afbfe38a83668e8473e7b1dbc85410a1eea7c20e`，已显式合入本地 develop `19d74cc7cbea49d73c3b4cab9139bb805dd886db`。专项隔离 MySQL、受影响 CI 分组、前端与构建验收记录保存在 Panel 的 `storage/framework/testing/orbit-codex-binding-evidence/`；它们只证明该 Panel 快照的声明出口，Orbit 消费与真实客户端载入分别需要自己的证据。

## 作用域规划

`orbit_core::codex_binding` 规范化账户出口，再由纯函数 `compile_scope` 编译目录。旧出口未带绑定时保持 `null`，损坏绑定只投影为不可用；推理策略必须保留支持集合、成员限制、明确默认值及真实的整数或空版本。它们不会阻止模型的其他用途，旧 `reasoning_efforts` 仍保留原有语义。

调用方必须提供已经按当前账户、节点和 provider 过滤的模型；规划器不从 UI 接收模板或授权声明。只输出同 provider 下 text、Responses、明确主模板绑定、已知 reasoning 支持与明确有效默认值的模型，顺序沿用 Panel。所选 alias 是目录的显式默认；其他未知或不兼容模型不猜测、不输出，所选模型不合格则整次规划失败。接线层要求 Panel 的默认 effort 非空并仍在有效交集中；通用纯编译器允许沿用模板默认的能力不能替代这一要求。

作用域摘要包含 origin、账户、安装、连接及其版本、授权 epoch、凭据版本、节点、provider、目录版本、所选模型、全部该范围模型的原始版本与规范化声明/策略，以及编译输出摘要。因此其他菜单项或作用域变化也会使旧预览失效。重复 alias、目录条目数量或编译输出上限超出时整组拒绝，不截断目录。摘要是比较冻结输入的一致性工具，不是新增授权或实际运行来源证明。

## 联合配置与恢复

Service 从当前账户、节点和 provider 的完整模型范围规划目录，预览仅向 UI 展示来源、摘要、路径、默认别名、reasoning 默认值和别名列表；完整模板及 journal 内容不经 IPC 返回。UI 只能提交单次 `preview_id` 和确认，不能提供模板、运行来源或授权材料。Panel 的 `verified: false` 声明与独立的 runtime 状态分别展示。

预览与应用入口先检查项目授权、实际 CLI 来源及组织策略，再刷新 bootstrap 和完整模型出口；bootstrap 刷新后重新检查节点、权限、密钥与连接版本。提交在账户锁内重编译整个目录，比较完整范围及输出，不能只复核所选模型。运行来源、策略或范围变化会永久退役旧预览，恢复原状态也不能复活它。

目录固定为配置同目录下的 `orivelle-orbit.models.json`。一次 journal 先发布目录，再发布 `config.toml`，将 `model`、`model_provider`、`model_catalog_json` 和 `model_reasoning_effort` 设置为本次明确结果，并沿用现有 helper 的认证合同。两个文件的存在状态和摘要共同构成配置版本；任一外部修改均使旧证据失效。指定 `profile` 或 profile 内覆盖上述字段时拒绝自动修改并要求人工核查。保留的 plan mode、其他配置文件、CLI/daemon 选择及其他 runtime 路径尚未全部验证，不能据此声称所有 reasoning 请求均受本配置覆盖。

加密 journal 冻结完整 actor、安装、连接、授权 epoch、凭据、节点、provider、模型声明与策略、来源与输出摘要、明确默认值、固定双路径及私有 runtime 身份。Vault 读取返回后复核来源与策略；解密后、每个文件写入前、授权启用前及提交恢复状态前复核完整上下文；先检查全部文件，发现第三种外部状态则不覆盖任何文件。旧 journal 缺少完整 Codex 上下文时不自动恢复。人工恢复只还原原始双文件字节，仍要求原 owner、固定路径、当前来源和策略；退出或推理密钥丢失后可恢复原文件，但不会复活旧 helper 授权。Claude 诊断导出也排除两个 Codex 目标，不能绕过 journal 覆盖目录。

生产 `CodexRuntimeSnapshot` 当前始终为 `unverified`，正向来源只存在于编译隔离的合成测试中。安装包版本、Panel 声明或 UI 状态不能构造私有的已验证身份。因此当前真实自动应用与恢复仍关闭；该接线只在明确的合成来源和假 Vault 下取得正向证据。

## 当前验收边界

项目内测试使用合成映射，验证模板保留、权限子集、引用、reasoning 与资源上限；作用域规划专项 10/10、双文件接线专项 10/10，以及导出目标保护的受影响 1/1 通过。最终完整 core128/helper1、rustfmt、helper 编译和 default/system typed check 通过；普通回归中的独立 signer 入口仍为 1 ignored，本阶段没有重复执行。前端完整质量链的 44/44 单测和 4/4 受影响浏览器用例通过，其他 11 项无变更流程未重跑。首轮浏览器在沙箱内启动阶段报告 SIGABRT，清理时报告 kill EPERM，四项页面断言未执行，原日志保留；成功运行使用项目内 profile 和 mock keychain。

最终源码、实际 argv、源码前后摘要、日志、历史失败及本地提交/合并记录见项目忽略目录 `.cache/evidence/codex-config-catalog-20261021/`。没有启动 Desktop、helper 或 Orbit GUI，没有读取用户认证资料、修改真实配置、调用系统安全存储或发送推理请求。

Panel 声明与纯目录编译不证明运行实例的 CLI 选择；组织远端策略仍为未知。自动配置仍需要独立的来源与策略门禁，不会据此生成 `loaded_evidence` 或 `connection_evidence`，也不代表实际模型菜单、流式或工具调用已经验收。现有 Mac 候选包仍绑定此前的源码，不能作为本模块的打包或运行证据。后续包含模板的新候选还需把固定来源的许可证和完整 NOTICE 纳入实际分发产物并回读验证。
