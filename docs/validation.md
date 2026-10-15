# 验证记录与环境门禁

此文件只记录已取得的证据，最终检查必须绑定提交后的完整 SHA。目标仍在实施中；任何未完成项都不构成平台支持或真实接入验收。

## 当前环境与授权

- 办公 Mac：macOS 26.7.1 arm64、Xcode 27.0、Node 24.18.0、npm 11.16.0、Rust 1.97.1。Docker 已启动。
- 项目内开发、依赖锁定、合成数据、隔离测试和本地提交已获准。原工作区和其他 worktree 保留。
- 真实客户端配置、实际系统安全存储测试尚未获准；真实凭据/上游/收费调用、生产数据库、签名证书安装、发布部署另行授权。
- `lab.lan` 仅个人 Mac 可用，当前不依赖它。Windows 11 25H2 x64 和 macOS 15 最低版本的实际环境尚无验收证据。
- macOS Intel 已取消，无此目标的设计、构建或验收。

## 分层记录

| 层级 | 当前证据 | 未覆盖 |
| --- | --- | --- |
| 前端语义 | format、零 warning lint、typecheck、27/27 单元测试、生产 build 与排除测试 fixture 检查通过；包括参考价来源/缺失语义、同账户正常刷新保留草稿、首次账户缓存缺失后的只读恢复、策略变化及原生拒绝后旧配置确认持续失效 | 真实 IPC 和目标客户端 |
| Desktop 协议 fixture | 项目内 Responses/Messages 网关 61/61；主/辅助模型、单/并行工具续轮、JSON/SSE、故障与资源预算；连同既有打包专项共 71/71 Node 测试通过，无跳过 | 真实 Desktop 的内置工具适配、实际请求/重试行为、HTTPS、模型菜单、载入与连接证据 |
| 浏览器界面 | Chrome 隔离 profile 15/15 通过；深浅色、390/1280/1600 宽度、键盘、弹窗焦点、无重发、单图删除、旧确认失效、授权拒绝/取消、会话刷新与过期、维护限制、无登录的冲突恢复预览；视觉复核已完成 | 原生窗口、安全存储、桌面应用接入 |
| Rust/native | 已有 CLT clang21 + SDK15.4：最新 core97/helper1 fake 回归、rustfmt、默认和嵌入 helper 的 system-integration typed cargo check通过；策略来源/晚变化、14组更新恢复故障夹具及图片出口、撤销、配额、授权、账务和锁回归通过 | 真实系统策略/vault/客户端文件/剪贴板/保存对话；不能将编译 feature 当成真实集成 |
| Panel | 阶段整合树独立复核六组2884项：2861 passed / 23 existing skipped、36897 assertions；隔离 MySQL8.4.11专项15/15，241 assertions；前端240/240及完整quality通过 | 后续其他任务推进的 develop 不属于此测试快照；真实 native/客户端/生产；未执行远端 GitLab pipeline |
| Edge | 本任务没有修改；合同以现有源码/测试核查 | 真实客户端与 fake gateway 的当前联调 |
| macOS arm64 | 当前 Mac release helper/GUI、`.app` 与 ZIP 构建成功；实际单 arm64、minOS15.0/SDK15.4、helper 完整内嵌、前端 custom-protocol 内嵌；未运行 GUI | macOS 15、原生真实运行和完整客户端流程 |
| Windows x64 | 无实际运行证据 | 编译、原生 vault/文件/ACL、客户端、安装与更新 |
| 发布候选 | Mac `.app` 与约9.2 MB ZIP 有完整包/归档 SHA-256、源码前后一致检查和解包回读；仅 linker ad-hoc，未设置 TeamIdentifier，正式签名/更新关闭 | 正式签名、公证、安装、更新失败恢复；Windows 候选 |
| 打包/CI 准备 | 2026-10-14 当前 Mac preflight/release/app/ZIP 实际成功；6/6候选证据专项、脚本格式/零warning lint/语法与5个 GitLab job YAML解析通过；项目镜像/cache与禁止 rustup自动安装保留 | Windows脚本与runner存在未核实，远端pipeline未运行 |

## 必需回归矩阵

登录覆盖 state/PKCE/issuer/install proof、授权码和票据单次消费、过期/撤销、refresh 旋转与重放、丢失响应或 vault 写失败后的重新授权、读取旧 session 不能与退出/重新授权竞争复活。

账户和接入覆盖 readonly/成员权限变化、节点不可用/URL变化、模型 revision、旧撤销确认、配置精确预览、外部修改、读回、受保护备份、各 journal 阶段崩溃恢复、无 key 仍能恢复原配置、helper 的不可变账户/安装/凭据绑定。

图片覆盖 UTF-8/完整 JSON 上限、真实格式与像素上限、多图完整保存 manifest、本地保存故障与重启恢复、单图删除、取消/断网/超时/坏结果/迟到结果、账户切换、真实零/缺失/混合币种、统一账务关联，并证明所有异常没有自动重发生成请求。

平台验证必须在 Windows 11 25H2 x64 和 macOS 15+ Apple Silicon 上分别记录编译、运行、配置、系统安全存储、拖放/文件/复制、两个真实桌面应用的模型选择、流式/工具/辅助模型和错误流程。CLI、跨编译、浏览器、Ubuntu或 fake vault 都不能替代这些验收。

## 尚待完成的实现

2026-10-15 策略来源采集、预览冻结及提交前重验已有项目内实现和合成测试；真实来源查询没有运行，Codex/Claude 的远端组织策略覆盖仍未取得证明，因此设备来源没有内容仍保持 `unknown`，自动配置继续受阻。来源范围和安全边界见[组织策略采集](managed-policy.md)。客户端载入/连接证据目前没有生产采集入口。已有项目内[协议隔离网关](desktop-fake-gateway.md)，覆盖 Responses/Messages 的合成主/辅助模型、流式、工具续轮和错误场景；尚未与真实 Desktop 联调，不产生加载或连接证明。Codex 的固定版本完整目录已作静态来源核查，目录编译和生产映射仍未适配。Claude library 的稳定合同和获准合成导出仍未取得；2.19675.0 安装包静态 reader/writer 显示 profile 为扁平对象，激活还涉及第三文件与 hybrid 来源指针，不能直接写入 nested V2 bootstrap body。自动 library 写入继续关闭。Mac 更新 journal、启动前校验、应用/数据库回退和受控终态退役已有项目内实现及故障夹具，Windows 更新恢复仍未实现为可验收流程，自动安装明确拒绝。

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
