# Codex 本机执行来源：纯判定层

2026-10-21 完成标准本机 backend 的参数解析、来源判定模型和界面范围文案。`codex_runtime` 仅在 `cfg(all(test, target_os = "macos"))` 下编译；没有生产运行来源采集器。生产 `CodexRuntimeSnapshot` 仍为 `unverified`。

2026-10-22 主线将[用户配置保存资格](desktop-configuration.md)与运行来源分开：保存依据受支持的已安装配置目标、明确目录和本地写入资格，结果仅为已保存、待重启。运行来源纯层不作为保存或恢复的活跃进程前置，未来实际载入和连接仍需独立证据。采集研究分支已单独保留，未并入本主线，也未进行第三次观察。

2026-10-21 用户要求该阶段告一段落后停止。纯判定层完成独立复核、分节提交及本地 `develop` 合并后已暂停；后续恢复及主线纠正以上述 2026-10-22 记录为准。

## 已查明的固定合同

公共安装包为 Desktop `26.1013.40934`（`com.openai.codex`），捆绑 CLI `0.163.0-alpha.10.2`。CLI 位于 `Contents/Resources/codex-cli/CodexCLI.app/Contents/MacOS/codex`，大小 249954368 字节，SHA-256 为 `1afb28c01d7d49e892bd75f11076b10f1451e34e0ee308a95e5ab7f69f9c84fe`。公开签名元数据的 TeamIdentifier 为 `2DC432GLL2`、identifier 为 `codex`；这些磁盘信息不证明某个运行实例已加载它。

对安装包公开 JS 的只读检查确定：标准本机 R5 路径使用 `app-server`、`--analytics-default-enabled`、`features.code_mode_host=true`，并包含两个插件布尔覆盖。插件值为 `false` 仍是合法结构。该固定 macOS 路径的覆盖使其不满足隐式 daemon 分支的无覆盖条件，按源码选择 stdio。自定义命令、其他 CLI、WebSocket、daemon 和未知覆盖不能据此放行。

子进程环境继承启动时的有效 `process.env`；桌面应用可能在启动后合入用户环境。实际子进程的 `HOME`、`CODEX_HOME` 和角色标志才是后续采集的依据，Orbit 自身环境或桌面应用最初的 exec 环境不能代替。Desktop 的空 `CODEX_HOME` 与 CLI 的回退语义不同，纯层对空值、非 UTF-8、相对路径和重复关键变量均拒绝推断。

本机 backend 的核验范围与前台选中的远端 host 分开；存在远端 host 本身不否定已独立证明的本机来源。此阶段没有证明任何运行 backend，也没有证明前台 host。

官方合同参见[网关接入](https://learn.chatgpt.com/docs/enterprise/connect-to-a-gateway)、[本机与云端模式](https://learn.chatgpt.com/docs/environments/modes)及 [App Server](https://learn.chatgpt.com/docs/app-server)。原始公开 JS、完整资源/签名报告及选择器锚点只保存在项目忽略目录 `.cache/evidence/codex-runtime-provenance-20261021/`；纯测试模块内置已复核的固定路径、大小、摘要及 CLI CDHash 常量，没有包含原始 JS 或生产采集器。

## 纯层完成范围

- 有界解析 KERN_PROCARGS2 形状的合成字节，只保留已知角色参数和四个已知环境键；未知环境值不解码、复制或格式化。必须存在环境区的空终止项，终止后的非零内容拒绝。
- 比较进程 PID、父进程、UID、启动时间、路径、文件实体、摘要、签名及前后快照；要求唯一候选和双向匹配的 AF_UNIX stdio 连接。模型中的签名、加载匹配和连接值目前均由测试构造。
- 比较实际请求根与目标根的规范路径、设备和 inode。revision 只纳入相关进程、来源、根和连接，排除配置/目录正文、根目录时间、无关 FD 和心跳，避免自身配置写入使来源失效。
- 六条双语文案明确“本机接入 / local backend”，不把 Panel 声明、磁盘版本或前台远端 host 当作运行来源。门禁、DTO 和配置行为未变。

Root 另用既有 CLT 在项目忽略目录编译只读探针，观察公开路径对应的进程元数据和 stdio FD 类型/双向 peer。探针没有读取实际 argv/env、管道内容、认证文件或配置，也没有运行 CLI、启动/重启客户端或访问 Keychain。观察到多个直接子 CLI 及 AF_UNIX stdio，仍缺角色、根和完整加载身份，结果明确为 `selected_runtime_proven: false`。

## 检查与证据

纯层提交 `d968a7b5f9073568d5e4672346ca6ee51b5ed65c`；文案提交 `c934aeef4ac6a436484bcfb80eef54584b86fbc8`。

`cargo test --locked --offline -p orbit-core --lib codex_runtime_` 实际通过 8/8，既有 128 项被过滤，未声称全量回归。覆盖角色/布尔值、未知覆盖、环境与路径、PID 重用及替换、stdio 唯一性、根变化和读取截断。独立复核发现的“最后一个已知环境项完整但缺少区终止符”问题已修复，并覆盖后续 `CODEX_HOME` 被截断的边界；首轮日志保留。最终两文件 rustfmt 检查及测试均 exit 0、无 warning，源码前后摘要相同。

文案的单文件 Prettier 实际执行后内容未变，ESLint `--max-warnings 0` 通过；受影响 Vitest 10 项通过、11 项因筛选跳过。没有修改测试断言，也没有重复浏览器或完整回归。当前运行的 `1426f16d` GUI 和旧候选不包含这些新文案，本阶段未重新打包或验收 GUI。

原始记录包括 `runtime-a-{targeted,fmt}-20261021-{1,2}.log`、`runtime-copy-checks-20261021-1.json` 及对应三份日志。只读探针首轮编译的符号比较警告导致 `-Werror` 失败，修正类型后编译/运行通过；全部历史记录保留。最终源码、日志摘要、独立审查及 Git 结果以该目录的 `codex-runtime-delivery-20261021.json` 为准。

## 封存采集研究的后续条件

2026-10-21 纯层交付时，生产采集器、FFI、实际 argv/env 读取和 Service 接线均未开始。恢复后的研究含测试专用采集及有限只读观察，已封存在独立分支，未合入当前主线。未来若启用正向运行身份，至少需要闭合这些已识别的条件；它们不是用户配置保存的活跃进程前置：

- 用真实、完整且有界的内核观察构造私有材料，原始启动缓冲使用 Zeroizing；公开错误和证据不输出未知参数、环境值或认证内容。
- 对运行代码进行签名有效性及磁盘匹配检查，并证明它加载的是已固定的资源代次。仅主 Mach-O/CDHash 一致不能证明运行 Electron 已加载当前 `app.asar`；应用存活期间资源被替换必须失效或取得等价证明。
- 对未知、读失败或截断的角色候选保持未核验，不能将其静默过滤后只选择另一个正常候选；多候选歧义也不能猜测。
- 在同一目标与配置根上下文中采集、绑定 stdio 及复核；保持现有预览、Vault 后、每文件写入前和恢复门禁。真实来源不能越过远端组织策略，也不生成载入、模型菜单或连接成功证明。

Windows、其他启动模式和完整真实客户端接入仍待相应实现与实际验收；本阶段的合成正例不授予这些能力。
