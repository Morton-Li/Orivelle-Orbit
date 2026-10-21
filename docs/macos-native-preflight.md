# 当前 Mac 的有限原生预检

2026-10-21，用户在真实客户端、配置和 Keychain 验收问题中明确答复：“本机就是Mac，直接用本机作为真实环境即可。” 当前办公 Mac 为 macOS 26.7.1 arm64。此记录只覆盖专用合成 Keychain 项及 Orbit 未登录原生界面，不构成整个 Mac 版本或第三方客户端的接入验收。

## 执行范围

首次启动 Claude、查看设置是为了核对外部配置入口；用户询问其目的后已停止其界面操作，改为先验证 Orbit 自身。本阶段未主动修改第三方配置、切换外部 profile 或提取认证信息，没有登录、真实上游、收费调用、系统信任、全局依赖或正式签名变更。静态 Claude 资料仅保留为后续线索，不作为真实配置载入证明。

功能分支为 `feature/orbit-macos-native-preflight`，起点为本地 develop `2e1972c9f52d3dce5da63b4dbfe7ef5b907d74ab`。本阶段只增加测试和验证文档，未改变产品或依赖；未重复已通过且无变更的全套回归和构建。编译、运行、失败、journal、截图、AX 和配置指纹记录位于 `.cache/evidence/macos-native-preflight-20261021/`，日志和本机路径材料不进入 Git。

## NativeVault 专项

`crates/orbit-core/tests/macos_native_preflight.rs` 仅在 macOS 编译，默认 `ignored`。它使用产品 `NativeVault` 的固定 service `io.orivelle.orbit` 和内部生成的 UUID 专用引用，不枚举 Keychain。初始值和更新值是内存中的随机 32 字节合成数据，以 `Zeroizing` 释放，不进入 argv、环境变量、journal 或断言输出。

任何 Keychain 查询前，测试先以 0600 创建并同步公共恢复 journal。只有该新引用的底层查询明确返回 `NoEntry` 才允许写入，并在首次写入前原子记录 cleanup 已启用。创建、读取、更新、删除、缺失及重复删除使用真实适配；补充的精确引用查询用于区分缺失和拒绝访问。RAII 只清理本次拥有的引用。

单目标 `--no-run` 编译和单文件 rustfmt 均通过，无警告。独立 reviewer 在实际执行前审查了完整源码。两次实际执行分别记录为：

| 执行 | 实际结果 | 清理 |
| --- | --- | --- |
| `native-vault-run-20261021-1`，受限环境 | exit101，create 失败，后续步骤保持 pending | 同引用删除与精确缺失确认通过 |
| `native-vault-run-20261021-2`，获准系统访问环境 | 同一已编译二进制，exit0，1/1 通过；10 步全部 passed | 删除、重复删除和精确缺失确认通过 |

首轮只记录了固定错误码，不能据此断言具体 OS 根因。二次运行二进制 SHA-256 为 `04d67703ebb4af29604858d7c5977759ddbcf9cc3522e5487b34b685a117e1fc`；测试源码 SHA-256 为 `2bd7470fae85a0c9df9858d8e85d34092acdca3c7058b69d3a707424fedec646`。独立 reviewer 核对了两轮日志与 journal，未运行额外 Keychain 查询。

显式重跑前应先确认当次本机授权，并准备项目内证据目录，沿用项目已有 Rust/CLT 和缓存环境：

```sh
mkdir -p .cache/evidence/macos-native-preflight-20261021
cargo test --locked --offline -p orbit-core --test macos_native_preflight --no-run
cargo test --locked --offline -p orbit-core --test macos_native_preflight native_keychain_roundtrip -- --ignored --exact --test-threads=1
```

普通 CI 不运行该用例。进程被强杀时 Drop 不会执行，必须按本次公共 journal 核对和处理该精确引用，不能假定清理成功。该专项证明当前测试二进制的真实适配往返；GUI/helper 的系统身份、升级和安全存储连续性仍需单独测试。

## Orbit 自身的实际运行

本次使用已存在的 Mac 候选，源码为 `1426f16dea242685296d08de9715fa1a4793789d`，GUI 为 22,218,832 字节，SHA-256 为 `3cd9c4624d9394bb2c278ede26263fb0d0634fe25d7d0dde0a50855a9cc51e64`。新增测试和文档不进入此 GUI，未重新打包或将当前分支冒充候选来源。

在真实 Tauri WKWebView 中取得以下观察：

- 首页、接入、图片工作台和设置均显示；Cmd+1、Cmd+2、Cmd+4 可导航。接入页显示当前安装的 Codex `26.1013.40934` 与 Claude `2.19675.0`，格式/策略仍待确认，自动接入禁用。
- 未登录时没有账户、节点或任务；图片生成和参考图输入禁用，本地历史为空。没有执行登录或图片调用。
- 脱敏诊断通过真实 IPC 生成，并经实际 macOS 保存对话框存入项目目录。导出内容确认 `native/fail-closed`、`logged_out`、0 连接、0 任务及客户端 `unconfirmed/not_tested`，未把声明的 Vault 模式作为 GUI Keychain 成功证据。
- Cmd+W 后 CUA 返回 `noWindowsAvailable (-10005)`；再次选择 Orbit 可重新显示此前的设置页和保存提示。随后回到首页留给用户查看。没有测量 PID，不据此增加进程身份结论。

首次启动在 `~/Library/Application Support/io.orivelle.orbit` 创建真实应用数据；此前该目录不存在。检查时目录均为 0700，数据库及 WAL/SHM 为 0600。部署的 helper 为 2,560,304 字节、0700，SHA-256 为 `8134169741d07cac8ba8078ae394dba8fd9697becd2dd4b75cf654a2392e3516`，与候选一致；本轮未执行 helper。

已记录的 Codex config.toml 与 Claude 主配置 SHA 在 Orbit 冒烟前后相同，新增 Codex 目录仍不存在，Claude library 的两个条目名称和长度未变。该初始快照是在 Claude 已启动后取得，且未对 library 正文建立前置 hash，不能证明 Claude 启动没有自行初始化或完整 library 字节未变。未创建含原配置的明文备份。

## 保留的验收缺口

登录/授权返回、GUI/helper Keychain、升级身份连续性、图片文件输入/生成/剪贴板、第三方配置切换/恢复/载入/模型菜单/流式/工具、HTTPS 联调和安装更新尚未完成。Codex 运行实例来源、远端组织策略和 Claude library 自动写入门禁没有被绕过。macOS15 最低系统和 Windows11 25H2 x64 仍缺真实验收环境；Intel 不在范围内。
