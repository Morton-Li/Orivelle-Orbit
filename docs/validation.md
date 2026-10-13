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
| 前端语义 | format、零 warning lint、typecheck、20/20 单元测试、生产 build 与排除测试 fixture 检查通过；包括参考价来源/缺失语义、同账户正常刷新保留草稿 | 真实 IPC 和目标客户端 |
| 浏览器界面 | Chrome 隔离 profile 12/12 通过；深浅色、390/1280/1600 宽度、键盘、弹窗焦点、无重发、单图删除、旧确认失效、授权拒绝/取消、会话刷新与过期；视觉复核已完成 | 原生窗口、安全存储、桌面应用接入 |
| Rust/native | 已有 CLT clang21 + SDK15.4：core43/helper1 fake 回归、默认和嵌入 helper 的 system-integration typed cargo check通过；撤销删除拒绝、旧资产作用域、10001条身份元数据与并发磁盘配额专项有证据；生产恢复/刷新/授权等闭包仍进行中 | 真实系统 vault/客户端文件；不能将编译 feature 当成真实集成 |
| Panel | 与最新 develop 整合后独立复核六组2884项：2861 passed / 23 existing skipped、36897 assertions；隔离 MySQL8.4.11专项15/15，241 assertions；前端240/240及完整quality通过 | 真实 native/客户端/生产；未执行远端 GitLab pipeline |
| Edge | 本任务没有修改；合同以现有源码/测试核查 | 真实客户端与 fake gateway 的当前联调 |
| macOS arm64 | 当前 Mac core/helper与完整 Tauri cargo check成功，未运行 GUI | macOS 15、原生真实运行和完整客户端流程 |
| Windows x64 | 无实际运行证据 | 编译、原生 vault/文件/ACL、客户端、安装与更新 |
| 发布候选 | 未使用正式密钥/证书，默认更新关闭 | 安装包、签名、更新失败恢复的最后产物证据 |
| 打包/CI 准备 | native-workflow 脚本语法、当前 Mac preflight、5个 GitLab job YAML解析与独立静态复核通过；锁定镜像、项目缓存、禁止 rustup自动安装、Windows大小写变量规范化、正式签名/渠道隔离 | Windows脚本与runner存在未核实；本轮未运行候选release build或远端pipeline |

## 必需回归矩阵

登录覆盖 state/PKCE/issuer/install proof、授权码和票据单次消费、过期/撤销、refresh 旋转与重放、丢失响应或 vault 写失败后的重新授权、读取旧 session 不能与退出/重新授权竞争复活。

账户和接入覆盖 readonly/成员权限变化、节点不可用/URL变化、模型 revision、旧撤销确认、配置精确预览、外部修改、读回、受保护备份、各 journal 阶段崩溃恢复、无 key 仍能恢复原配置、helper 的不可变账户/安装/凭据绑定。

图片覆盖 UTF-8/完整 JSON 上限、真实格式与像素上限、多图完整保存 manifest、本地保存故障与重启恢复、单图删除、取消/断网/超时/坏结果/迟到结果、账户切换、真实零/缺失/混合币种、统一账务关联，并证明所有异常没有自动重发生成请求。

平台验证必须在 Windows 11 25H2 x64 和 macOS 15+ Apple Silicon 上分别记录编译、运行、配置、系统安全存储、拖放/文件/复制、两个真实桌面应用的模型选择、流式/工具/辅助模型和错误流程。CLI、跨编译、浏览器、Ubuntu或 fake vault 都不能替代这些验收。

前端最新证据为 `.cache/evidence/frontend-auth-refresh-check.log`、`ui-auth-refresh-e2e.log`，深色创作与历史截图均有独立文件。Rust 证据为 `runtime/logs/native-core-tests.log`、`native-tauri-check.log`、`native-tauri-system-check.log`；候选构建尚未运行。Panel 整合证据为其任务 worktree `storage/framework/testing/orbit-integration-evidence/summary.json`、`six-groups.json`、`quality.json`、`mysql-direct.log` 与 JUnit/events；旧 `orbit-evidence/` 保留。日志与缓存不入 Git。

Panel 集成 feature SHA `ab787c0cdeb694e75c6d71a1524fb9256515ef02` 已显式 --no-ff 合入本地 develop `0a8bd6331d90ec4e495df8855bb49506fe08728d`，两者树一致。原订阅/retention24文件保留，原工作区 `.demo/` 和任务记录未改。任务 worktree 在相同 feature HEAD detach，功能分支保留；未推送或合 main。

2026-10-13 最新指令：本轮原生代码阶段收口后暂停目标，实际候选打包与系统集成留到恢复目标以后。目标仍未完成，不将缺少环境/授权的项目标成验收通过。

阶段说明：前端源码阶段已通过独立复核；后端适配阶段亦已通过独立复核。分步骤 local commits 后显式 --no-ff 合入本地 develop，再开下一阶段分支。Native 尚未完成的文件不夹入前端阶段提交。已有 Panel bundle-size 提示、浏览器环境 NO_COLOR 提示按实保留，不能称所有工具零警告。Chrome 首轮沙箱启动 SIGABRT/EPERM，使用获准 project-profile/mock-keychain 后测试才实际运行。
