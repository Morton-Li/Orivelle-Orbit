# 原生检查与候选打包

当前脚本用于项目内编译和候选产物审查，不启动原生应用、不运行真实客户端或系统安全存储测试。候选的正式签名与更新渠道关闭。真实运行、客户端配置、安全存储、正式签名和分发另行授权。

## 已准备的工具

只接受当前宿主匹配的 `aarch64-apple-darwin` 或 `x86_64-pc-windows-msvc`，不交叉编译。Node 24.18.0、npm 11.16.0、Rust 1.97.1、Tauri CLI 2.12.2 及目标标准库、rustfmt 应由环境负责人预先准备。脚本没有安装工具的步骤。

`ORBIT_RUST_BIN` 可以指向已准备工具链的 bin；默认读取用户已有的 `.rustup/toolchains/1.97.1-<宿主>/bin`，直接执行锁定 cargo/rustc，避免调用 rustup 自动下载。macOS 默认使用现有 CommandLineTools clang 与 MacOSX15.4 SDK；如准备路径不同，通过单次命令的 `ORBIT_DEVELOPER_DIR`、`ORBIT_MAC_SDK` 指定。不会执行 xcode-select、接受 Xcode 许可证或修改全局环境。Windows 需已有 MSVC、Windows SDK 和已配置的构建终端。

依赖使用项目 npmmirror/rsproxy 源和锁文件完整性校验。Cargo、npm、临时目录、编译结果及日志均留在项目 `.cache/`、`runtime/`。镜像缺少锁定版本时停止报告，不降级、不自动换官方源。

## 分阶段命令

```sh
node scripts/native-workflow.mjs preflight
npm run check
node scripts/native-workflow.mjs check
node scripts/native-workflow.mjs signer-probe
node scripts/native-workflow.mjs candidate
node scripts/native-workflow.mjs bundle
```

`preflight` 只核工具、宿主和项目路径。`check` 执行 rustfmt、core/helper 的隔离测试以及默认 Tauri cargo check；此构建拒绝真实 vault。

`signer-probe` 要求已提交、干净的 tracked 源码，实际调用锁定 CLI 2.12.2：在项目私有临时目录生成两对合成密钥，签署小型假产物（分别带 `--app-version` 和不带版本），随后显式运行 `updater_cli` 的 Rust verifier 测试。正例使用原签名、版本和相同字节；负例包括字节、公钥、版本、无版本签名、trusted comment、编码和空产物，CLI 另须拒绝 TAB/CR/LF 版本。普通 `cargo test` 将该入口标为 ignored；两个 CI native validate job 显式运行 probe，不能把 ignored 当作已执行。

所有 signer stdout/stderr 在内存中有界捕获，不写日志；合成私钥不放 argv，不读取用户签名密钥，不使用系统证书。临时目录在成功和失败后删除，测试编译使用独立 `runtime/target-signer-probe/`；候选仍无 release trust。成功摘要 `runtime/logs/synthetic-updater-signer-probe.json` 仅包含版本、公开 hash、状态和源码前后一致检查。失败的新尝试先移除旧成功摘要，不能用旧记录代表新运行。

专用 target 复用项目目录门禁，已有 symlink/junction 会在启动工具前被拒绝。超时或输出超量后停止保留输出，只终止本次 owned process group，强制结束后仍以有限 settle 进入清理，不无限等待继承 pipe。2026-10-14 Mac 故障夹具4/4通过：符号链接、不退出的 signer、父进程退出后子进程仍持 pipe、输出超量；并验证临时密钥和旧成功摘要被删除、既有 sibling 保留。命令 `node --test tests/build/updater-signer-probe.test.mjs`，证据 `runtime/logs/signer-probe-failure-tests.log`。Windows 仅按 owned PID 调用系统 [taskkill /T /F](https://learn.microsoft.com/en-us/windows-server/administration/windows-commands/taskkill)，环境名规范化；进程树及故障行为仍需实际 Windows 验证。

2026-10-14 已在办公 Mac 实际通过该 probe，首轮源码 `cdb6f2d50dd2b67329604cb948a72b4108f14231` 前后一致；软件签名的合成产物为96字节、版本0.2.0，Rust显式测试通过，临时目录残留0。首轮摘要与日志单独保存在 `.cache/evidence/synthetic-updater-signer-probe-cdb6{.json,-run.log}`；阶段收尾重新执行后，最新源码和结果以 `runtime/logs/synthetic-updater-signer-probe.json` 为准。这证明锁定 CLI 与版本/字节 verifier 的互通，未验证正式签名、目标架构、安装或更新。CLI 的写钥与签名格式依据固定源码：[generate](https://github.com/tauri-apps/tauri/blob/5b23c82ea5ba6b9aec420dd71f60493edc486aa9/crates/tauri-cli/src/signer/generate.rs)、[版本签名](https://github.com/tauri-apps/tauri/blob/5b23c82ea5ba6b9aec420dd71f60493edc486aa9/crates/tauri-cli/src/helpers/updater_signature.rs)。

`candidate` 先构建带 `system-integration` 的固定 helper，再通过 `ORBIT_BUNDLED_HELPER_PATH` 把同一二进制及 SHA-256 嵌入 GUI。构建脚本检查项目内普通文件、大小和目标架构；运行时部署使用独占 helper 锁、私有稳定路径、原子写入和读回校验。helper 与桌面程序都只在构建时生成；脚本不会启动它们。

`bundle` 在 macOS 生成 `.app`，不挂载 DMG、不放入 Applications。Windows 生成当前用户 NSIS 安装候选，需先从获准镜像准备完整 NSIS 3.11 和固定 0.5.3 plugin 到项目的 `runtime/target/.tauri/NSIS/`；缺文件或 plugin 校验和不符即停止，避免 Tauri 自行从 GitHub补下载。候选 NSIS 使用 `webviewInstallMode=skip`，要求测试环境已有 WebView2；正式安装的运行时交付另做审查，不能把这个候选当成可分发安装包。

macOS 同时生成 `runtime/candidates/Orivelle-Orbit-0.1.0-macos-arm64-candidate.zip`（名称随版本变化）。归档只包含 `.app`，排除扩展属性、资源 fork 和 ACL；在项目临时目录解包，再核对全部文件、目录、权限和内容摘要，验证后清理该临时副本。原 `.app` 和 ZIP 保留在项目内，未执行或安装。

Tauri `useLocalToolsDir` 将工具缓存限定到项目 target，行为以锁定 CLI 源码为据：[配置](https://v2.tauri.app/reference/config/#bundleconfig)、[2.12.2 NSIS 实现](https://github.com/tauri-apps/tauri/blob/tauri-cli-v2.12.2/crates/tauri-bundler/src/bundle/windows/nsis/mod.rs)。当前没有 Windows 打包或系统运行证据。

产物摘要位于 `runtime/logs/candidate-manifest.json`。构建前要求已提交、干净的 tracked 树，记录 HEAD/tree；完成包检查后再次核对，变化时拒绝生成有效摘要。文件负责人在构建期间保持源码冻结；任务记录和测试缓存不入 Git。

摘要包含宿主/工具版本、helper/主程序大小与 SHA-256；macOS 增加 bundle identifier、版本、最低系统、完整包清单/摘要、ZIP 摘要及解包一致性结果。包清单按路径排序，包含文件/目录类型、POSIX 权限、文件大小与内容 hash；整体 hash 为该清单 JSON 的 SHA-256，不包含时间戳或系统额外属性。它证明本次候选的内容与构建提交对应，正式发布信任仍需签名与渠道流程。

锁定 CLI 2.12.2 的 `--no-sign` 会跳过 macOS 的 keychain、证书签名和公证分支，也跳过 updater 签名：[固定实现](https://github.com/tauri-apps/tauri/blob/5b23c82ea5ba6b9aec420dd71f60493edc486aa9/crates/tauri-bundler/src/bundle/macos/app.rs#L115)。实际 Mach-O 仍有编译链接器产生的 ad-hoc 签名，未设置 TeamIdentifier，Info.plist/资源未封印；这不能证明发布者身份或安全存储的升级连续性。正式证书、公钥与更新渠道未启用。

Mac 证据模块的专项回归命令为 `node --test tests/build/macos-candidate.test.mjs`。覆盖资源/权限变化、可执行文件替换、标识/版本/最低系统不符、路径与 symlink、构建前 dirty 和中途提交变化；Mac CI 运行这些检查并保留 ZIP。实际 `.app`、架构、helper 嵌入和归档回读另行静态核实，均不会启动 Orbit 或 helper。

## CI 与验收

GitLab 配置要求准备好的 `orbit-macos-arm64`、`orbit-windows-x64` 原生 runner，以及已安装的 Chrome。这些 tag 是环境需求，尚未核实 runner 实际存在；没有推送或触发远端 pipeline。CI 只做编译、FakeVault/fixture 和隔离浏览器检查，Windows NSIS 是准备好专用缓存后的手动候选 job。

macOS 15 最低系统、Windows 11 25H2、真正的原生 vault/ACL、配置加载、两个真实桌面应用的工具/流式/辅助模型、安装及更新失败恢复都需要分别记录实际证据。构建或浏览器通过不能替代这些门禁。
