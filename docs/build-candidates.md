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
node scripts/native-workflow.mjs candidate
node scripts/native-workflow.mjs bundle
```

`preflight` 只核工具、宿主和项目路径。`check` 执行 rustfmt、core/helper 的隔离测试以及默认 Tauri cargo check；此构建拒绝真实 vault。

`candidate` 先构建带 `system-integration` 的固定 helper，再通过 `ORBIT_BUNDLED_HELPER_PATH` 把同一二进制及 SHA-256 嵌入 GUI。构建脚本检查项目内普通文件、大小和目标架构；运行时部署使用独占 helper 锁、私有稳定路径、原子写入和读回校验。helper 与桌面程序都只在构建时生成；脚本不会启动它们。

`bundle` 在 macOS 生成 `.app`，不挂载 DMG、不放入 Applications。Windows 生成当前用户 NSIS 安装候选，需先从获准镜像准备完整 NSIS 3.11 和固定 0.5.3 plugin 到项目的 `runtime/target/.tauri/NSIS/`；缺文件或 plugin 校验和不符即停止，避免 Tauri 自行从 GitHub补下载。候选 NSIS 使用 `webviewInstallMode=skip`，要求测试环境已有 WebView2；正式安装的运行时交付另做审查，不能把这个候选当成可分发安装包。

Tauri `useLocalToolsDir` 将工具缓存限定到项目 target，行为以锁定 CLI 源码为据：[配置](https://v2.tauri.app/reference/config/#bundleconfig)、[2.12.2 NSIS 实现](https://github.com/tauri-apps/tauri/blob/tauri-cli-v2.12.2/crates/tauri-bundler/src/bundle/windows/nsis/mod.rs)。当前没有 Windows 打包或系统运行证据。

产物摘要位于 `runtime/logs/candidate-manifest.json`，包含源码 SHA、tracked dirty、宿主/工具版本、helper 和 executable 的大小/SHA-256，明确记录未启动、无正式签名、无实际集成验收。必须在提交后的干净 tracked 树重建摘要，才能绑定最终候选；`AGENT.md` 和测试缓存不入 Git。

## CI 与验收

GitLab 配置要求准备好的 `orbit-macos-arm64`、`orbit-windows-x64` 原生 runner，以及已安装的 Chrome。这些 tag 是环境需求，尚未核实 runner 实际存在；没有推送或触发远端 pipeline。CI 只做编译、FakeVault/fixture 和隔离浏览器检查，Windows NSIS 是准备好专用缓存后的手动候选 job。

macOS 15 最低系统、Windows 11 25H2、真正的原生 vault/ACL、配置加载、两个真实桌面应用的工具/流式/辅助模型、安装及更新失败恢复都需要分别记录实际证据。构建或浏览器通过不能替代这些门禁。
