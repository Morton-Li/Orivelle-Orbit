# Orbit 运行架构与安全边界

Orbit 是 Panel 的桌面客户端及本地接入协调器。Panel 决定账户权限、节点/模型目录和结算；
Edge 执行模型请求。Orbit 管理桌面授权、本地配置、Claude 转发、图片文件及只读用量投影。
本文描述当前源码职责，不作为应用已加载配置、真实上游成功或发布验收的证据。

## 进程与模块职责

| 层                | 职责与入口                                                                                                                                                        |
| ----------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Vue / Pinia UI    | 展示原生状态、收集用户选择和确认，通过[typed bridge](../../src/shared/bridge.ts)调用命令；不持有凭据或直接访问 Panel/Edge                                         |
| Tauri 桌面壳      | [commands](../../src-tauri/src/commands.rs)提供闭合 IPC、原生文件选择/剪贴板和更新动作；[应用入口](../../src-tauri/src/lib.rs)组装 Service、事件、窗口及单实例    |
| orbit-core        | [Service](../../crates/orbit-core/src/service.rs)协调账户作用域、网络、vault、store、配置事务及图片任务，返回[公开 DTO](../../crates/orbit-core/src/dto.rs)       |
| credential helper | [独立可执行文件](../../crates/orbit-helper/src/main.rs)只接受客户端类型和 opaque binding UUID，从固定部署位置定位只读 store，再按冻结授权读取凭据或本地映射 token |

主窗口 capability 仅开放声明的 Orbit 命令；CSP 将前端网络限制为 IPC 和本地图片预览。
`orbit://state`、授权、错误、图片输入及下载进度事件用于刷新视图；UI 的状态和按钮不代替
原生校验。异步操作在账户、连接 revision 或授权 epoch 改变后拒绝发布旧作用域结果。

桌面构建支持 macOS arm64 与 Windows x64。启用 `system-integration` 的构建使用应用数据
目录、真实用户路径和系统 vault，并校验部署的 helper 摘要；普通隔离构建使用仓库 runtime
路径和 DeniedVault。开发 UI 或测试替身不能据此取得真实安全存储及用户配置写权限。

## Panel 桌面授权与凭据

生产 Panel origin 由[network](../../crates/orbit-core/src/network.rs)固定为
`https://orivelle.powerbyte.io`；普通 debug 构建也不开放 issuer 覆盖。其他 issuer、回环
HTTP 和测试 CA 只存在于显式编译的测试客户端。

[Installation](../../crates/orbit-core/src/identity.rs)为安装生成 Ed25519 密钥及备份加密密钥。
登录先证明 vault 可读，再启动随机端口的 `127.0.0.1` 回调监听和 PKCE S256，打开同源
Panel 授权页，由系统浏览器完成用户登录/确认。原生回调严格核对 Host、路径、state、issuer
和单次授权请求；超时、取消或被后一次登录替代的流程不能提交连接。

换码及后续桌面 API 使用安装证明，绑定方法、目标、安装/会话、时间、nonce、request ID、
正文摘要及需要时的 Bearer 摘要。会话 access/refresh token 与推理 `orv_sk_*` 分开存放；
推理凭据经一次性 delivery ticket 领取，未知交付结果不自动重领，账户仍可保留只读访问。
节点/模型、API key 状态和权限由 Panel 刷新；客户端不从目录缓存猜测新的推理授权。

登出先推进本地授权失效状态、停用 helper binding 和 Claude 映射，再尽力撤销远端会话。
清理 vault 失败不会恢复已失效授权。helper 按 origin、安装、账户、provider、凭据 revision、
epoch、有效期和 enabled 状态逐项匹配，不跟随当前 GUI 选择切换到其他账户。

## vault、本地存储与启动恢复

[vault](../../crates/orbit-core/src/vault.rs)使用 macOS Keychain 或 Windows 原生凭据存储，
保存签名私钥、会话、推理凭据、备份密钥及 Claude 本地 token；机密不进入公开 IPC DTO。
helper 只有对应读取路径，Claude 映射 secret 的新建能力单独注入 GUI，不开放通用 ACL 更新。

[Store](../../crates/orbit-core/src/store.rs)使用私有应用目录和 SQLite WAL，保存连接/安装
元数据、设置、任务及文件索引。图片任务包含本地提示词等业务资料，参考图、原图和预览
保存在私有本地文件；它们不因使用 vault 而成为端到端加密数据。配置原始字节进入加密 journal。
图片路径、格式、摘要及预览请求均由原生层校验，WebView 只按当前作用域读取受控 asset。

单实例与 runtime lock 避免多个桌面进程共同改写同一目录。启动先处理更新恢复，再打开
Store 和 Service；中断的图片任务标为 `unconfirmed`，不重放网络生成。配置恢复另按已确认
journal 协调；恢复不能确认时保留冲突并阻止同客户端新应用，不擅自覆盖外部改动。

## 配置预览、应用与切回

[SystemPaths / 检测](../../crates/orbit-core/src/system.rs)核对安装身份、所需配置能力、
默认用户目录及[本地托管策略](../../crates/orbit-core/src/policy.rs)。自动配置当前只支持
macOS 默认路径；Windows 的本地配置接入保持不可用。未知来源、目录锚点或策略不能授权写入。
配置资格不等价客户端正在运行、已读取配置或实际连通，也不依赖一个固定版本白名单放行。

预览冻结账户、安装、节点/模型目录、凭据与策略 revision、目标应用和文件 before/after 摘要，
返回一次性、有期限的 preview ID。用户明确确认后，原生层刷新远端事实，再重检同一上下文。
预览期间客户端更新、权限变化或文件冲突要求重新预览；不能只凭 UI 旧预览直接写入。

[ConfigEngine](../../crates/orbit-core/src/config.rs)用客户端锁、原子文件替换及持久
`prepared → publishing → committed/conflict` journal 协调整组文件与 helper grant。
原始字节以 XChaCha20-Poly1305 加密，关联安装、账户、客户端、策略及操作用途。
每个客户端维护独立的接入前切换快照；切回也先预览并确认，已有外部修改时暂停覆盖。
恢复旧配置字节不会复活已登出、撤销或过期的 helper 授权。

## Codex 与 Claude 的本地链路

Codex 配置与模型目录通过同一事务发布：[catalog binding](../../crates/orbit-core/src/codex_binding.rs)
将当前账户/节点允许的模型声明与包内受控模板结合，保留推理权限交集，并确定默认模型。
配置只写入节点地址、目录及 helper 引用；Codex 调用 helper 取得绑定凭据后直接访问 Edge。
目录声明、配置已保存和 `restart_required` 状态均不证明 Codex 已加载或上游请求成功。

Claude 的配置库、活动索引和必要主设置由[Claude 协调器](../../crates/orbit-core/src/service_claude.rs)
一起处理。当前接入要求分别选择 Sonnet/Opus/Haiku 模型，显示名称记录映射关系，实际请求
使用所选模型 ID；这三个入口不是失败时换模型的调度策略。

[mapping manager](../../crates/orbit-core/src/claude_mapping.rs)为绑定保留固定 loopback
端口及专属本地 token。helper 向 Claude 提供本地 token；[HTTP 映射](../../crates/orbit-core/src/claude_mapping_http.rs)
只接收固定模型目录/Messages 路由，核对 Host、Bearer、别名及当前授权，拒绝浏览器来源头、
任意目标和升级连接。core 替换模型并携带真实推理凭据访问既定 HTTPS Edge，流式返回结果。
每次转发有有界并发/体积/期限和原生 permit；不跟随重定向、不自动重试已发送请求，不开放
通用代理。Orbit 必须持续运行；重启后只为经重新核验的当前活动配置恢复映射。

## 图片任务与平台用量

[图片模块](../../crates/orbit-core/src/images.rs)限制格式、尺寸、解码内存、提示词、请求/
响应体和本地磁盘预算。创建任务冻结账户/连接、节点、模型及输入摘要；发送前再检查当前
授权，只向选定 Edge 的 Images generation/edit 端点发送一次。当前产品每次一张结果，
最多五张参考图；UI 只开放已支持的背景控制，不把通用 API 参数全集当作产品能力。

提交前可取消排队；提交后的取消、超时、网络故障或进程退出保留结果待确认，不能推断
上游已撤销或免计费。收到图片后先暂存再提交本地历史；`retry_local_save`只保存已有结果，
不重新生成。已取得 Request ID 的任务可以通过 Panel 只读记录核对执行/费用，不凭空补图。
复制和另存先冻结文件摘要，最终写入时重查账户、epoch 和历史归属。

[用量服务](../../crates/orbit-core/src/service_usage.rs)用桌面会话及安装证明读取 Panel 汇总
和分页记录，覆盖该账户全部平台入口；只读账户不需要推理凭据或已选节点。查询和返回时
均检查当前连接/revision；金额保持十进制字符串及币种，未知、待确认和历史语义原样保留，
Orbit 不计算平台价格、余额、预算或结算，也不以本地图片历史替代服务端账务。

## 更新与桌面生命周期

[更新模块](../../crates/orbit-core/src/updates.rs)只有编译入审核过的渠道/公钥才启用应用内
更新，Windows 当前使用手动安装包。下载与安装分别确认；候选须匹配平台、更高版本、
签名版本及归档边界，下载 URL/重定向限于固定渠道规则，不能由 WebView 或设置提供信任根。

安装进入 maintenance，排除在途任务、配置操作与授权，并独占 helper 更新锁。
[恢复事务](../../crates/orbit-core/src/update_recovery.rs)封存旧包/数据库基线，替换应用后
重启；下次启动在暴露 Service 前验证并提交数据，失败按 journal 恢复旧包和受控数据状态。
恢复后要求重启或发现篡改冲突时，不继续用已映射的新二进制提供普通服务；不借数据库回退
恢复旧凭据有效性，也不回放模型请求。

[窗口/托盘生命周期](../../src-tauri/src/desktop.rs)与 Service 分离：默认关闭窗口只收起，
后台图片任务和 Claude 映射继续；真正退出停止映射，未确认图片按上述状态保留。
设置可将关闭改为退出，存在任务、未保存结果或转发时提示影响；托盘不可用时保留窗口入口。
再次启动唤回已有实例。普通网络、错误及[诊断导出](../../crates/orbit-core/src/service.rs)
只公开受控状态，原始网络错误/凭据不直接进入 UI 或导出；用户须先预览再选择保存诊断。
