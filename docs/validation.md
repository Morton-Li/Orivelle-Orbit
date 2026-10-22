# 验证记录与环境门禁

此文件只记录已取得的证据，最终检查必须绑定提交后的完整 SHA。目标仍在实施中；任何未完成项都不构成平台支持或真实接入验收。

## 2026-10-22 Claude 配置事务引擎

从本地 develop `becfaae8` 建 `feature/orbit-claude-config-transaction`，分别提交认证上下文 `5f8d62d`、整组保存与逆序恢复 `85e3646`。本节只完成原生事务引擎；Service 的预览、确认、启动恢复和客户端状态接线仍待下一分支完成。Orbit 直接对接 Panel，继续使用其授予的现有节点和模型，本节没有新增网关或后端服务。

事务专项最终 25/25 通过，包含每次写入与授权前整组状态检查、全部最终文件读回、旧新绑定缺失时恢复原字节，以及安全存储返回后的到期与撤销复核。独立审查发现的源 journal 晚变和已提交操作重新进入时的晚变读回两个边界已修复，并新增反例通过；早期版本 1/2 的检查证据保留。

最终回归绑定源码 `85e364623c8da3a19b4b520c0b6fd983b819cc71`：core 168/helper 1 通过，core 内 2、独立 Vault 1、signer 1 保持 ignored，完整 Rust 格式检查通过。131 个源码和测试文件前后摘要不变。生产库编译保留一组 `prepare_claude`、`claude_context` 尚待 Service 使用的 dead_code warning，未压制该提示。证据位于 `.cache/evidence/claude-config-transaction-20261022/`；没有重跑无变更的前端、Panel 授权或协议网关，没有重建候选，也没有操作真实客户端配置、安全存储或界面。

## 2026-10-22 Claude 配置基础阶段

从本地 develop `425af149` 建 `feature/orbit-claude-config-library`，分节提交固定合同、纯三文件合并、界面确认和安装目标/本地策略资格。Claude 生产保存与恢复入口仍关闭；后续另建分支接入 typed journal、Service 和启动恢复。Panel 继续是账户、凭据、节点和模型的权威；本阶段没有新增 Sub2api 网关或 Edge 服务。

固定公开安装材料静态复核与签名检查通过，没有启动 Claude 或读取其用户配置。纯合并 5/5、目标/策略合成 6/6 通过；独立审查发现的签名缓存 P2 已修为每次完整收据重新验签，仅复用 ASAR 哈希，首轮日志保留。界面单元 51/51、类型、受影响格式与 lint 通过；五项 Claude 浏览器流程最终通过。首次受限环境浏览器启动出现 SIGABRT，清理时报 kill EPERM，未执行页面断言；获准复验四项通过、一项重复状态标签定位失败，仅限定该断言区域后单项通过，所有失败记录保留。

冻结原生/界面源码 `53d0be0e8a67c8faf7c0948b4f55c47cbd95e494` 后，完整 core 155/helper 1 通过，core 2、独立 Vault 1、signer 1 保持 ignored；fmt、helper build、前端生产构建与产物检查、default/system typed check 全部 exit 0，无编译 warning。130 个源码/测试文件前后摘要不变。已通过且无新变更的界面 51 项及五个浏览器流程、Panel 授权、协议网关与签名探针不重复执行。确切命令、日志及分段清单位于 `.cache/evidence/claude-config-library-20261022/`；此阶段不重建候选，不代表真实 Claude 载入、Code 请求、系统策略/Vault/配置或 Windows 验收。

## 当前环境与授权

- 办公 Mac：macOS 26.7.1 arm64、Xcode 27.0、Node 24.18.0、npm 11.16.0、Rust 1.97.1。Docker 已启动。
- 项目内开发、依赖锁定、合成数据、隔离测试和本地提交已获准。原工作区和其他 worktree 保留。
- 2026-10-21 用户明确授权当前办公 Mac 作为真实集成环境，有限原生预检已验证 Orbit 自身和专用合成 Keychain 项；用户询问提前打开 Claude 的目的后，已停止第三方客户端界面操作。2026-10-22 主线转回用户配置预览、保存与恢复；运行来源诊断研究分支保留但未合入。真实凭据/上游/收费调用、生产数据库、正式签名证书安装、发布部署仍需另行授权。
- `lab.lan` 仅个人 Mac 可用，当前不依赖它。Windows 11 25H2 x64 和 macOS 15 最低版本的实际环境尚无验收证据。
- macOS Intel 已取消，无此目标的设计、构建或验收。

## 2026-10-22：真实 Panel 隔离授权

用户允许 Docker 临时环境后，当前范围收紧为 **Orbit → Panel 的登录与账户接入**。没有搭建 Sub2api 网关、Edge 或推理上游。后端固定 Panel `01d8f67a5af6ba189c933a6f214de5ecfb8010e3` 的源码快照，实际运行 PHP-FPM **8.5.12**、MySQL **8.4.11**、队列和 Scheduler；入口仅发布回环 TLS 端口。基础镜像固定 digest，PHP-FPM 通过镜像源取得，其余镜像和匹配锁文件的项目依赖缓存复用。源码快照不继承 Panel `.env`，Docker 客户端配置、构建缓存、证书和合成凭据均在项目内；系统信任、hosts 和全局工具未改。

默认 ignored 的 macOS 组件入口实际运行 **1/1 通过，2.90 秒，145 filtered**。它调用真实 `Service::start_login`、安装签名、S256 PKCE、token 交换和推理 key 交付，不使用 scripted HTTP 或直接注入账户会话。Node driver 经实际 Fortify 密码登录和明确授权页批准；登录和同意两处缺少 CSRF token 都返回 419。回调发送前匹配本次实际 listener，state/issuer 继续由原生 Service 校验。严格 TLS 正例、错误 CA、错误 SAN 和其他 origin 拒绝均通过，没有跳过证书或名称验证。

两次登录之间先退出桌面会话，再证明相同安装关联的同一推理 key 被复用；未确认撤销被拒，明确撤销后账户读取仍可用，最后退出的旧会话被拒。只读 MySQL 回执实际观测到 **1 个安装、2 个已撤销桌面会话、1 个已撤销专属 key**，合成初始余额仍为精确字符串 `10.00000000`，上游账户、Edge 机器凭据、执行和账本记录均为 0。回执未选择 key、token、hash 或秘密列。

模型证据是公开目录过滤及固定 Codex 绑定：一个公开合成模型可见，一个非公开模型不可见，不能泛化为完整成员授权矩阵。节点如实保持离线，原生 Service 拒绝选用；配置写入也保持禁止。本入口使用项目 Store、MemoryVault 和空客户端清单，未运行共享默认配置或 NativeVault 往返，未启动候选 GUI 或第三方客户端。真实浏览器界面、GUI 登录、可用 Edge、推理计费、配置保存后的客户端载入与连接仍须分别验收，不能称为完整上线验收。

本阶段受影响 Rust 回归为 **core 144 / helper 1**，格式、helper 编译、默认与 `system-integration` typed check 全通过，无编译 warning。普通回归的两个 core 集成入口保持 ignored，NativeVault 与 signer 专项各一个 ignored；随后只显式执行上述 Panel 入口。前端产品代码无变化，没有重复此前已绿的前端或浏览器流程。新 Node 脚本的格式、语法和 scoped ESLint 检查通过。

保留的首轮失败包括 buildx 尝试写全局活动目录被限制、合成 seed 未识别固定迁移自带的 custody 禁用记录，以及 Caddy 文件 capability/临时目录权限和纯内部网络下的入口发布问题。修复限定在临时 fixture：项目 Docker 配置、精确迁移基线检查、非 root 入口的必要 capability 和私有 tmpfs、仅入口加入发布 bridge。只重试失败的 seed 或入口启动，没有重跑已通过的构建和迁移；未清库隐藏失败。

复现入口见[临时 Panel 环境](../tests/acceptance/panel/README.md)。源码、日志和完整性回执在 `.cache/evidence/panel-local-20261022/`；实际后端、首次失败及 MySQL 回执在 `.cache/panel-acceptance/20261022054057-1eba3f/`。最后的分步提交、源码对应关系、资源清理和本地合并以本阶段交付 JSON 为准。此前 `f108f93` Mac 候选保持独立来源，本阶段未重打包或新增其 GUI 运行证据。

## 分层记录

| 层级 | 当前证据 | 未覆盖 |
| --- | --- | --- |
| 前端语义 | 2026-10-22 配置保存合同：format、零 warning lint、typecheck、56/56 单元测试、生产 build 与 fixture 排除检查通过；完整检查发现缺少新字段的默认视图，补为未核验后从失败处继续通过 | 新版本未在原生 GUI 载入；目标客户端实际接入；有限原生 IPC 见下行 |
| Desktop 协议 fixture | 项目内 Responses/Messages 网关 61/61；主/辅助模型、单/并行工具续轮、JSON/SSE、故障与资源预算；连同既有打包专项共 71/71 Node 测试通过，无跳过 | 真实 Desktop 的内置工具适配、实际请求/重试行为、HTTPS、模型菜单、载入与连接证据 |
| Codex 目录与接线 | 固定 CLI0.163.0-alpha.10.2 完整官方目录的12项编译用例、10项作用域用例和10项双文件接线用例通过；已消费 Panel 显式绑定及真实整数 reasoning revision，联合 journal 覆盖配置、目录和授权恢复 | 运行实例来源、真实 Desktop 载入/模型菜单与能力、全部 runtime reasoning 路径 |
| Codex 保存与运行来源 | 新保存合同使用已安装配置目标、本地写入资格和确认后固定目录创建；全局策略、运行来源及载入/连接证据各自保留未知。早期纯来源层与未合入的采集研究均不提供实际载入证明 | 真实默认文件保存/恢复、客户端载入及连接待专项；未知运行来源不再作为保存前置条件 |
| 浏览器界面 | 2026-10-22 Chrome 隔离 profile 的4/4受影响模型预览、失效确认、冲突恢复和Claude导出用例通过，其余11项无变更流程未重跑；此前完整15/15及深浅色、390/1280/1600宽度和视觉记录保留 | 原生窗口、安全存储、桌面应用接入 |
| Rust/native 合成回归 | 2026-10-22 CLT clang21 + SDK15.4：core144/helper1、rustfmt、helper 编译、默认与 system-integration typed cargo check通过；NativeVault 和 signer 专项各1 ignored。随后仅给首次目录权限测试增加 macOS 注解，该单例及格式复验通过 | 此结果仅为 Mac 合成与编译证据；Windows 配置目标及相关 synthetic fixture 尚待适配，不能泛化为 Windows 通过 |
| 本机 NativeVault | 新增默认 ignored 的 macOS 专项；单目标编译和格式通过。受限环境首轮 create 失败并清理通过；获准环境以同一已编译测试二进制实际运行 1/1 通过，新随机项的 10 步 journal 全 passed | GUI/helper 的身份与 Keychain 连续性、拒绝访问及平台全流程；首轮具体系统原因未确认 |
| 本机配置往返入口 | 2026-10-22 新增默认 ignored 的 Service 专项，真实默认配置与四个专用 NativeVault 引用的执行代码已通过独立复核、单目标 `--no-run` 编译和格式检查；写前恢复记录、碰撞不读取秘密、冲突保留恢复能力及固定 panic 输出已复核 | 用户选择先做本地候选与 Docker 隔离验收，因此本次没有运行该入口，没有新增真实配置或 Keychain 往返结果 |
| Orbit 原生界面 | 1426f16d 源码候选在 macOS 26.7.1 arm64 的 Tauri WKWebView 中实际运行：四页、快捷键、未登录限制、诊断 IPC、原生保存对话及关闭窗口后重新显示通过 | 登录、图片输入/生成/复制、helper 执行、第三方客户端接入、最低 macOS15 与 Windows；详见[本机预检](macos-native-preflight.md) |
| Panel | 绑定阶段源afbfe38a：隔离MySQL13/13（660 assertions）；三个受影响CI分组、前端329/329、PHP及前端quality通过，历史单项失败和复验见下文；最终安全构建179文件独立核对 | 真实 native/客户端/生产；未执行远端 GitLab pipeline；后来推进的 develop 不属于此快照 |
| Edge | 本任务没有修改；合同以现有源码/测试核查 | 真实客户端与 fake gateway 的当前联调 |
| macOS arm64 | 历史42900ddd源码的 release helper/GUI、`.app` 与 ZIP 构建成功；新候选由bundle流程绑定完整源码、单arm64/minOS15.0、helper嵌入和三份Codex来源材料；1426f16d 候选已完成上行有限原生运行 | macOS 15 和完整客户端流程 |
| Windows x64 | 无实际运行证据 | 编译、原生 vault/文件/ACL、客户端、安装与更新 |
| 发布候选 | Mac `.app` 与 ZIP 要求完整包/归档 SHA-256、源码前后一致检查和解包回读；具体版本、大小和来源材料hash以candidate-manifest为准；仅 linker ad-hoc，未设置 TeamIdentifier，正式签名/更新关闭 | 正式签名、公证、安装、更新失败恢复；Windows 候选 |
| 打包/CI 准备 | 2026-10-14 当前 Mac preflight/release/app/ZIP 实际成功；6/6候选证据专项、脚本格式/零warning lint/语法与5个 GitLab job YAML解析通过；项目镜像/cache与禁止 rustup自动安装保留 | Windows脚本与runner存在未核实，远端pipeline未运行 |

## 必需回归矩阵

登录覆盖 state/PKCE/issuer/install proof、授权码和票据单次消费、过期/撤销、refresh 旋转与重放、丢失响应或 vault 写失败后的重新授权、读取旧 session 不能与退出/重新授权竞争复活。

账户和接入覆盖 readonly/成员权限变化、节点不可用/URL变化、模型 revision、旧撤销确认、配置精确预览、外部修改、读回、受保护备份、各 journal 阶段崩溃恢复、无 key 仍能恢复原配置、helper 的不可变账户/安装/凭据绑定。

图片覆盖 UTF-8/完整 JSON 上限、真实格式与像素上限、多图完整保存 manifest、本地保存故障与重启恢复、单图删除、取消/断网/超时/坏结果/迟到结果、账户切换、真实零/缺失/混合币种、统一账务关联，并证明所有异常没有自动重发生成请求。

平台验证必须在 Windows 11 25H2 x64 和 macOS 15+ Apple Silicon 上分别记录编译、运行、配置、系统安全存储、拖放/文件/复制、两个真实桌面应用的模型选择、流式/工具/辅助模型和错误流程。CLI、跨编译、浏览器、Ubuntu或 fake vault 都不能替代这些验收。

## 尚待完成的实现

2026-10-22 按用户主线纠正实施[用户配置保存合同](desktop-configuration.md)：保存依赖受支持的已安装目标和完整本地写入资格，不要求已启动的运行实例。远端组织策略未知仍保持未知；任何已知管理限制、无法读取的本地来源或失效目标均拒绝写入。UI、默认视图修复和后端合同已经分节提交；新原生往返入口已编译并复核，按本轮隔离范围未执行。Mac 候选按最终提交构建，实际源码、包与归档结果以 `runtime/logs/candidate-manifest.json` 和本阶段交付清单为准。

用户确认没有外部 Panel 测试入口，要求先完成本地候选与隔离验收，并明确允许 Docker 临时环境。下一阶段采用真实 Panel、Edge、MySQL 和合成账户、假上游，验收授权、安装关联 key、节点/模型、受控请求、账目及撤销；先解决项目或单进程 CA 信任，保持完整证书链和主机名校验，不改系统信任或 hosts。Docker/API 结果与候选 GUI、真实 Desktop 载入/连接及各平台系统验收分别记录，不能相互替代。

2026-10-21 [测试专用纯判定层与范围文案](codex-runtime.md)是已合入本地 `develop` 后暂停的历史阶段。恢复后运行来源采集研究已保留在独立分支，没有合入当前配置主线，也没有第三轮真实来源观察。旧 `1426f16d` 候选保持独立归档，不代表本次代码。

2026-10-21 当前 Mac 授权后的有限原生预检已取得真实 NativeVault 单项及 Orbit 未登录界面证据，过程与边界见[本机预检](macos-native-preflight.md)。这不改变运行实例来源、远端组织策略、Claude library 写入、第三方 Desktop 载入/连接和其他平台的剩余门禁。此前各阶段的“未运行/未获准”陈述只描述当时快照。

2026-10-21 来源材料候选阶段从本地develop `d8f5ac9f7450a2999839e5dbee86825496730ccd` 创建独立分支，只补固定Codex许可证/NOTICE/provenance的实际包保留及候选证据。打包前源码必须已提交且tracked干净；构建、三文件与ZIP回读、静态检查、完整源码SHA及本地合并结果见 `.cache/evidence/codex-candidate-resources-20261021/`。旧42900ddd ZIP/manifest保留；旧429源码尚不含完整目录及双文件接线，不能代表后来的代码。1426f16d 候选已包含目录接线，其后新增文案及本次保存合同仍须新候选验证。此阶段不重新运行无变更的Rust/前端、浏览器、协议网关或signer探针，也不启动真实程序。

2026-10-15 策略来源采集、预览冻结及提交前重验已有项目内实现和合成测试；真实来源查询没有运行，Codex/Claude 的远端组织策略覆盖仍未取得证明，因此设备来源没有内容仍保持 `unknown`，该历史阶段的自动配置继续受阻；2026-10-22 的 Codex 用户文件写入资格见上文。来源范围和安全边界见[组织策略采集](managed-policy.md)。客户端载入/连接证据目前没有生产采集入口。已有项目内[协议隔离网关](desktop-fake-gateway.md)，覆盖 Responses/Messages 的合成主/辅助模型、流式、工具续轮和错误场景；尚未与真实 Desktop 联调，不产生加载或连接证明。2026-10-21 已实现固定版本完整目录、Panel显式绑定消费、Service/config和目录联合恢复，见[目录与接线](codex-catalog.md)；生产runtime仍为unverified，没有实际来源生产入口。Claude library 的稳定合同和获准合成导出仍未取得；2.19675.0 安装包静态 reader/writer 显示 profile 为扁平对象，激活还涉及第三文件与 hybrid 来源指针，不能直接写入 nested V2 bootstrap body。自动 library 写入继续关闭。Mac 更新 journal、启动前校验、应用/数据库回退和受控终态退役已有项目内实现及故障夹具，Windows 更新恢复仍未实现为可验收流程，自动安装明确拒绝。

2026-10-21 从develop `e7c7072b50abd0b5dd39928bbadb0656968c64bb` 建立 `feature/orbit-codex-config-catalog`。作用域规划提交 `d8c783806ff71262d532d8089cf27b5aba8ad1a0`；界面与固定错误分节提交 `2def6e62dd2e8292cd80c0fb79a4e2f00339a4cd`、`972095eb3a7f2578f9c2f8a677104344f129851b`。独立审查关闭了bootstrap仅刷新模型导致节点/权限/密钥陈旧，以及Claude导出可能覆盖新增目录的两项P2；补齐受影响反例，原强断言保留。双文件上下文覆盖完整模型范围，失效预览永久退役，旧无上下文journal不猜测恢复，退出后原字节恢复不会复活旧授权。

本阶段最终Rust五个门禁实际exit0：完整core128/helper1、rustfmt、helper build、default/system typed check，源码前后摘要相同且无warning。前端完整check为44/44，受影响浏览器4/4；首轮浏览器在沙箱内启动阶段报告SIGABRT、清理时报告kill EPERM，四项页面断言未执行；获准的项目profile/mock-keychain复验才实际通过，失败日志保留。源码未变的其他浏览器流程、协议网关、signer探针与打包未重复运行。实际argv、环境覆盖、源码/日志摘要和本地合并以 `.cache/evidence/codex-config-catalog-20261021/` 内最终交付清单为准。当前生产runtime正向身份只允许测试构造；编译与合成成功不会开启真实自动配置，也不生成loaded/connection证据。

Panel绑定阶段从develop `6281359f6739bee613e2183ece078cf7850f16db` 开始，源 `afbfe38a83668e8473e7b1dbc85410a1eea7c20e` 显式合入本地develop `19d74cc7cbea49d73c3b4cab9139bb805dd886db`。受影响CI：platform803通过、1项409失败后稳定构建上单项复验1/1；data1395通过/17既有skip；database88通过。409发生时Vite正在改manifest，与用例冻结Inertia版本冲突是源码及时间推断，未捕获实际响应头；不记作一次804/804。专项SQLite首轮15通过/9失败的raw snapshot差异修正为fresh()->getRawOriginal后，仅复验9/9；不记作一次24/24。隔离MySQL13/13含实际旧版升级、全新安装、重复迁移及回退保护。

该Panel快照的最初标准Vite构建按项目默认读取了.env，日志保留并标为隔离/产物选择已替代。最终使用真实空envDir、剥离继承VITE变量的安全构建，179个文件独立核对；manifest摘要 `44d5deda0935ee4786f0925c8226d5cbae9a86c47b21e72f8bb988aea36d308d`，REVISION为afbfe38a。Composer、Pint、PHPStan及前端format/lint/type、329/329单测和产物专项通过。项目临时.env.testing经owner/hash检查后移除，本任务loopback/tmpfs MySQL容器已按ID/标签核对停止删除；真实.env未编辑。完整事实与旧日志见Panel `storage/framework/testing/orbit-codex-binding-evidence/codex-binding-delivery-20261021.json`；不代表生产、原生或后来提交。

2026-10-21 用户明确恢复后，从干净 develop `9576fafed7fe758740fd2fe80654d9c968285d93` 建立 `feature/orbit-codex-catalog`。完整原始目录及许可证/NOTICE/来源单独提交 `8e0a7049e28828372de56a510a07b3ed1904219e`；纯编译 API 和 12 组用例提交 `e6e49c42f2fcd8b502b523b780319c4533d547b3`。当前公共安装包为 Desktop26.1013.40934/CLI0.163.0-alpha.10.2，官方固定提交 `4ebc485ca321243727539187169425fcf972f2c4` 的目录与内嵌字节仅差 LF；旧 0.162 证据保留，未当作当前版本。原始数据按完整 JSON 值保留，未通过有损 ModelInfo DTO 重建。编译调用方声明版本、授权和辅助依赖，编译器不提供这些声明的外部真实性证明。

本阶段最少检查实际通过：目录专项12/12、完整 core109/helper1、rustfmt、helper 编译、default/system typed check；没有重复无变更的 UI、浏览器、协议网关、签名探针或打包。普通 Cargo 回归中原有 CLI signer 入口仍为1 ignored，本阶段未另行执行。日志为 `runtime/logs/catalog-{targeted,final-fmt,final-core-helper,final-helper-build,final-default-check,final-system-check}-20261021-1.log`，源码和日志摘要及最终本地合并以 `.cache/evidence/codex-catalog-delivery-20261021.json` 为准。独立源码、用例及来源审查无本阶段 P1/P2。没有实际执行 Desktop/GUI/helper、读取认证资料、访问 Vault、修改用户配置或调用上游，先前 42900ddd 候选包不能代表本模块的打包或运行证据。

2026-10-15 协议隔离网关阶段从 develop `67f1cd65d685097042b1a0aa32c117ac9ec95db1` 创建 `feature/orbit-desktop-fake-gateway`，按基础 fixture、CI、工具/连接修复、故障与边界用例、文档分组提交。模块源码提交为 `80db039d525d17c5f9062572611a1020529c5c5d`，三文件及八份日志绑定 `.cache/evidence/fake-gateway-final-source-20261015.json`。网关最终 61/61、含既有打包专项的全 Node 71/71、前端完整 check 的 27/27 单测通过；格式、lint、语法及 YAML 五个 job 解析通过。两平台验证 job 均已接入，远端 pipeline 和 Windows 未运行。

独立审查已关闭工具续轮角色/顺序/前缀、未交付 pending 清理、socket/pipeline/Expect 预算与计数、Responses 文本字段及测试代理隔离问题。所有 HTTP 测试客户端显式直连；子进程开启环境代理选项并设置仅回环的假代理，仍直连成功且 stderr 为空。取消用例证明已终结旧请求的重复 abort 不影响新 pending；没有声称暂停旧 finally 并制造实际穿插。首轮关闭后 socket 未归零的两项失败保留在 `fake-gateway-foundation-1.log`；最终日志 `fake-gateway-final-3.log`、`fake-gateway-final-root-all-node-20261015.log`、`fake-gateway-final-root-frontend-check-20261015.log` 均为实际通过。

此阶段未改应用/Native 源码，未重复浏览器或 Rust 回归；最新应用候选仍绑定 `42900ddd52949a9440bed4f2e9df4d18e38e3efc`，稳定 ZIP 为 `runtime/candidates/Orivelle-Orbit-0.1.0-macos-arm64-42900ddd-candidate.zip`，源码、签名探针和产物证据见 `.cache/evidence/managed-policy-delivery-20261015.json`。隔离网关使用固定合成工具 `orbit_fixture_read`，没有内置到客户端候选，也不代表真实 Desktop 工具执行、TLS、系统安全存储或平台验收。阶段最终 SHA/合并/同 HEAD detach 以 `.cache/evidence/desktop-fake-gateway-delivery-20261015.json` 为准。用户本轮要求收尾后暂停，未进入下一开发阶段。

受管理策略阶段从 develop `c88deee1b8feeb26c43b31f234a85683cd4fbdf4` 创建 `feature/orbit-managed-policy`。界面分组提交 `1194ae0f28db2b8245a49bc36ed1a40c0a266663`、`f13ec7d07ece156c5e2aa00c3e99025a3fc6277b`，随后来源采集 `a190ef9` 与配置门禁 `dc49faea5e8fbb98c1e673bb0926ce4bd6c86382` 分组提交。Native 最终 97 core + 1 helper 通过，普通回归的 CLI signer 入口仍为 1 ignored；fmt、helper 编译、default/system typed check 通过。最终五文件及六份日志绑定 `runtime/logs/native-managed-policy-final-source.json`，原始失败和历史日志也在清单内。首次 launcher 127 未启动 Cargo；之后两项旧 fixture 缺少新身份/策略材料导致失败，修正后保留旧无策略 journal 拒绝断言，最终全部通过。

本阶段独立 reviewer 提出的检查后文件替换/FIFO 阻塞、普通 CF 偏好误报为强制管理、耗时读取后的最终守卫和旧预览复活均已修复并补测。界面最终 27/27 单测、15/15 浏览器通过，对应 `runtime/logs/managed-policy-ui-{check,e2e}-final-3-20261015.log` 与 `.cache/evidence/managed-policy-ui-final-source-20261015.json`。Chrome 首次沙箱启动失败未执行页面断言，其原日志和 trace 保留；成功复验使用项目内 profile 与 mock keychain。所有结果来自合成来源和假 Vault，未执行真实 CFPreferences/registry 或 Desktop。最终源码、候选包和本地合并以阶段交付证据为准，旧 aff8 候选保持独立归档。

锁定 Tauri CLI 的合成签名互通已补齐：2026-10-14 在干净提交 `cdb6f2d50dd2b67329604cb948a72b4108f14231` 上实际执行 CLI2.12.2 的版本/无版本签名、TAB/CR/LF拒绝和同一 Rust verifier 的正负例，运行exit0、源码前后一致，临时合成私钥目录残留0。该首轮摘要与日志保存在 `.cache/evidence/synthetic-updater-signer-probe-cdb6{.json,-run.log}`；收尾重跑的最终源码以 `runtime/logs/synthetic-updater-signer-probe.json` 为准。合成信任不进入候选 release，此测试不等于正式证书、真实安装或更新验收。

2026-10-14 更新恢复阶段：从 develop `848a1113add4fced18e7864f7769b47e1611ba15` 建立 `feature/orbit-update-recovery`，按签名互通、进程/缓存边界、核心恢复、原生安装/启动接线和文档分节提交。14组故障夹具包含交换前后、私有验证失败、main/WAL/SHM发布中断、回退重入、授权失效、foreign材料、终态清理中断、配置journal、悬空pending、claimed live S2恢复S1，以及未认领S2完整保留。最终回归80 core + 1 helper，普通回归中的 CLI fixture入口仍为1 ignored，必须另外显式运行 signer-probe。实现顺序与边界见[更新恢复](update-recovery.md)。

该阶段日志为 `runtime/logs/native-update-recovery-final-{core-tests,fmt,helper-build,default-check-2,system-check}.log` 与 `native-update-recovery-fault-tests-5.log`。首轮只读原库检查生成sidecars的失败、配置夹具基线顺序引起的SHM readmark差异和首轮default编译warning均保留在此前日志；配置夹具将snapshot revision读取移至raw family基线之前，完整main/WAL/SHM断言保持不变。Root 的6候选 + 4 signer故障测试合计10/10，无跳过；脚本format/lint、GitLab五个job的YAML解析和前端check23/23通过。最终提交、日志hash、Mac候选的源码/归档摘要和本地合并事实以 `.cache/evidence/update-recovery-delivery-20261014.json`、`runtime/logs/candidate-manifest.json` 为准；历史8e95候选另存 `runtime/candidates/previous/`。

2026-10-14 图片出口阶段：`feature/orbit-image-export-safety` 从候选阶段本地 develop `43022f006ad991bc5adaf0d8ce809f779cb5c40f` 建立。独立 reviewer 发现并关闭复制/另存为的晚写入和合法文件替换问题；最终 66 core + 1 helper、rustfmt、helper 编译、default/system-integration typed check通过。证据为 `runtime/logs/native-image-export-final-{tests,fmt,helper-build,default-check,system-check}.log`。新测试只使用 FakeSink/FakeVault 和项目内文件，覆盖解码/读取后的退出、账户/origin/连接/revision/epoch变化、过期、忘记安装、删除、维护、hash及写入锁序；未运行 GUI、真实 helper、系统剪贴板、vault 或文件对话框。8e95 的 Mac ZIP 不含本阶段修复，不能用作新代码的候选证明。

图片出口阶段4次小提交最终为 `8928e78c90346203e89090562f1950eb7feeca21`，显式 --no-ff 合入本地 develop `848a1113add4fced18e7864f7769b47e1611ba15`，树一致；4个源码文件与5份日志的SHA已独立复核。任务同HEAD detach、分支保留后，从该 develop 新建 `feature/orbit-update-recovery`。原仓和其他任务未改，未push/main/tag。合并前系统Python触发Xcode许可门禁，未执行合并；随后使用现成Node和子进程CLT完成实际合并，没有接受许可或改系统配置。

前一源码阶段证据为 `.cache/evidence/frontend-final-check.log`、`ui-final-e2e.log`（23 unit / 15 browser）；首次账户缓存缺失专项另存 `frontend-cache-recovery-check.log`。深色创作与历史截图均有独立文件。此前 Rust 证据为 `runtime/logs/native-core-tests.log`、`native-tauri-check.log`、`native-tauri-system-check.log`（57 core / 1 helper）；8e95 首个候选构建阶段没有修改 Rust/应用 TS 源码。最终源码 SHA 与收尾检查绑定记录保存在未跟踪任务记录和项目证据目录。Panel 整合证据为其任务 worktree `storage/framework/testing/orbit-integration-evidence/summary.json`、`six-groups.json`、`quality.json`、`mysql-direct.log` 与 JUnit/events；旧 `orbit-evidence/` 保留。日志与缓存不入 Git。

2026-10-14 Mac 候选证据：`runtime/logs/macos-candidate-build-20261014.log` 首次 release、`macos-first-bundle-build-20261014.log` 首次 `.app`、`macos-verified-bundle-build-20261014.log` 含完整包/ZIP 回读的重建、`macos-candidate-evidence-tests-20261014.log` 的6/6专项，以及 `candidate-manifest.json`。最终提交后再次构建，完整 SHA 以最后 manifest 的 `source_head` 为准；归档为 `runtime/candidates/Orivelle-Orbit-0.1.0-macos-arm64-candidate.zip`。没有启动 helper/GUI、访问真实 vault、修改真实客户端或运行付费请求。

实际静态检查：两个 Mach-O 均为单 arm64，最低15.0、SDK15.4；动态加载路径仅系统 Frameworks 与 `/usr/lib`，无本机 Homebrew/worktree 依赖或 LC_RPATH；GUI 包含与独立 helper 完全一致的字节和 hash。实际 tauri/macros release fingerprints 启用 custom-protocol，Context 为 dev=false，使用内嵌 dist。正式证书/公证未做，资源未封印；文件 hash 与 bundle identifier 不构成正式应用身份或实际系统安全存储验收。

浏览器复验曾为 14/15：Vite 把正在修改的 Rust 文件视为全页重载来源，备份账户选择随页面重载清空，恢复按钮用例超时。WebSocket trace 明确记录 `service.rs`、`commands.rs` 后的 `full-reload`。已仅排除 Native 源码和项目缓存的前端文件监听，保留 Vue/TS 监听及原断言，完整重验为 15/15。原失败日志 `ui-cache-recovery-e2e.log` 和 `.cache/evidence/ui-cache-recovery-failure/` 中 trace 保留，不将失败改记为通过。

Panel 集成 feature SHA `ab787c0cdeb694e75c6d71a1524fb9256515ef02` 已显式 --no-ff 合入本地 develop `0a8bd6331d90ec4e495df8855bb49506fe08728d`，两者树一致。原订阅/retention24文件保留，原工作区 `.demo/` 和任务记录未改。任务 worktree 在相同 feature HEAD detach，功能分支保留；未推送或合 main。

后续另一任务把 Panel develop 推进到 `86fb1e1e1e2320dcbbba297d9d6781a4bf1f701b`，上述阶段合并仍为其祖先；本任务没有覆盖、回退或重新验收另一任务的变更。暂停收尾时仅停止了本任务空闲 MySQL 容器。它原以 `--rm` 启动，Docker 自动删除容器和匿名测试卷；测试证据、fixture 和项目缓存完整保留，恢复后需重建隔离数据库，其他容器未改。

2026-10-13 原生阶段已按指令合入 Orbit develop `518534196c6d1d8f5cf942e42455d61cee3f6415` 后暂停；原 native feature `f2274f50fdad30002d2c3322878a18888ee7ecc1` 保留。2026-10-14 用户明确继续，从该 develop 建 `feature/orbit-candidates`；用户确认 Windows 尚未准备，本轮先完成 Mac 候选，真实系统集成仍待另行授权。目标仍未完成，不将缺少环境/授权的项目标成验收通过。

2026-10-14 只读兼容核查观察 Panel develop `33298533d675c39f611cc9d70ab7eb7fa9d7dc01`、Edge develop `0cfa719a32cbf7064fe425d4242d1474bc75a023`。Orbit auth/session/proof/key/账户数据及公共 fixture、图片 handler/adapter/metadata 未变；新增子账户退休撤销沿用既有生命周期/401合同，未发现阻止 Mac 候选的 JSON 破坏。没有修改或重新运行这两个仓的测试；旧2861/23、MySQL15、frontend240证据仍只属于原 Panel feature，不能宣称覆盖新 develop。

阶段说明：前端、Panel 和原生源码阶段已通过独立复核，分步骤本地提交并显式 --no-ff 合入本地 develop。Mac 候选阶段继续相同节奏，产物只留项目内，正式更新渠道、证书签名与实际安装/更新仍关闭。已有 Panel bundle-size、浏览器 NO_COLOR 和预期 skipping-signing 提示按实保留，不能称所有工具零警告。Chrome 首轮沙箱启动 SIGABRT/EPERM，使用获准 project-profile/mock-keychain 后测试才实际运行。
