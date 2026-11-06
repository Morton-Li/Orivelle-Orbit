# 开发、检查与候选打包

## 前端开发

在 Orbit 仓库根目录使用 `.node-version` 指定的 Node 24.18.0 和 `package.json` 指定的 npm 11.16.0。运行 `npm ci` 按锁文件安装项目依赖，`.npmrc` 配置了 npm 镜像与项目内缓存。

```sh
npm ci
npm run check
npm run test:e2e
```

`npm run check` 检查格式、Lint、类型、单测及前端产物。`npm run test:e2e` 使用已安装的 Chrome，在空闲的回环端口 1420 启动 Vite，并注入合成 IPC 数据覆盖界面流程。手动预览前端可用 `npm run dev`，浏览器里的原生操作需要桌面 IPC。独立协议测试见[桌面协议隔离网关](desktop-fake-gateway.md)。

## 原生工具链

| 构建宿主 | Rust 目标 | 平台前置 |
| --- | --- | --- |
| macOS Apple Silicon | `aarch64-apple-darwin` | 默认使用 Command Line Tools 的 clang 和 macOS 15.4 SDK。可用 `ORBIT_DEVELOPER_DIR` 与 `ORBIT_MAC_SDK` 指向已准备的工具目录和 SDK。候选配置的最低系统版本为 macOS 15.0。 |
| Windows x64 | `x86_64-pc-windows-msvc` | 使用已配置 MSVC x64 与 Windows SDK 的终端。运行桌面程序需要 WebView2。 |

原生脚本要求宿主架构和 Rust 目标一致。准备 `rust-toolchain.toml` 固定的 Rust 1.97.1、对应目标标准库和 rustfmt。脚本从 `ORBIT_RUST_BIN` 或用户已有的 `.rustup/toolchains/1.97.1-<目标>/bin` 调用实际二进制。Tauri CLI 2.12.2 随 `npm ci` 安装。Cargo 使用项目配置的 rsproxy sparse 镜像和 `Cargo.lock`。[原生工作流脚本](../scripts/native-workflow.mjs)

前端和原生检查请使用没有本地 dotenv 文件的 checkout，避免 Vite 将本地配置带入产物。原生脚本为子进程隔离 npm 配置和缓存，清除继承的 Vite、签名和发布变量，并将 Cargo 缓存、构建输出和临时文件写入 `runtime/`。依赖缓存齐全时可设置 `CARGO_NET_OFFLINE=true`。

```sh
node scripts/native-workflow.mjs preflight
node scripts/native-workflow.mjs check
node --test tests/build/updater-signer-probe.test.mjs
```

`preflight` 核对宿主与锁定工具版本，准备项目目录。`check` 执行 rustfmt、core/helper 常规测试和默认特性的 Tauri `cargo check`。默认特性的应用使用项目内数据目录、隔离客户端路径和拒绝访问的 Vault。

## 生成候选产物

在已提交且无跟踪文件改动的 checkout 中运行签名探针，再按所需产物选择 `candidate` 或 `bundle`。

```sh
node scripts/native-workflow.mjs signer-probe
node scripts/native-workflow.mjs candidate
node scripts/native-workflow.mjs bundle
```

- `signer-probe` 单独执行常规测试中标为 `ignored` 的更新签名互通测试。锁定的 Tauri CLI 用临时合成密钥签署小型产物，Rust verifier 核对产物字节和签名版本，并运行拒绝用例。私钥在测试后清理，成功摘要写入 `runtime/logs/synthetic-updater-signer-probe.json`。
- `candidate` 构建带 `system-integration` 的原生程序，向主程序嵌入同目标的项目内 helper 和它的 SHA-256。编译产物位于 `runtime/target/<Rust 目标>/release/`。运行这类候选会使用系统应用数据目录及原生集成。
- `bundle` 在同一次构建中执行平台打包。macOS 在目标目录下的 `bundle/macos/` 生成 `Orivelle Orbit.app`，并在 `runtime/candidates/` 生成 `Orivelle-Orbit-<版本>-macos-arm64-candidate.zip`。Windows 在 `bundle/nsis/` 生成当前用户范围的 NSIS 安装候选。

Windows 执行 `bundle` 前，需在 `runtime/target/.tauri/NSIS/` 预置完整的 NSIS 3.11 和 `Plugins/x86-unicode/additional/nsis_tauri_utils.dll` 0.5.3。脚本校验必需文件与插件 SHA-1，缺项会在调用打包器前失败。请按锁定版本源码中的资源地址与校验和，从可用镜像准备工具。NSIS 候选设置 `webviewInstallMode=skip`，安装环境须预装 WebView2。[Tauri 2.12.2 的 NSIS 工具定义](https://github.com/tauri-apps/tauri/blob/tauri-cli-v2.12.2/crates/tauri-bundler/src/bundle/windows/nsis/mod.rs#L33-L69)

构建成功后，`runtime/logs/candidate-manifest.json` 记录前后核对的 Git 提交和树、宿主与工具版本，以及 helper 和主程序的大小与 SHA-256。macOS 打包还核对 bundle 标识、版本、最低系统版本、完整文件清单和 ZIP 解包一致性。`.app` 与 ZIP 中的固定 Codex `LICENSE`、`NOTICE`、`PROVENANCE.json` 都须通过路径和内容校验。可用 `node --test tests/build/macos-candidate.test.mjs` 运行打包检查的夹具测试。[候选证据实现](../scripts/candidate-evidence.mjs) · [Codex 目录来源](codex-catalog.md)

候选通过 `--no-sign` 构建，更新端点为空。发布准备包括平台签名与 macOS 公证。启用自动更新还需配置更新签名和可信渠道。真实平台、客户端和系统安全存储的验收范围见[验证指南](validation.md)。

## macOS 安全存储专项

`native_keychain_roundtrip` 是默认忽略的真实 Keychain 测试。确认当前环境允许写入一条合成条目后，在 Orbit 根目录执行。直接调用 Cargo 时请使用上述已准备的 Rust 和 SDK，并使用已填充的项目 Cargo 缓存。

```sh
export CARGO_HOME="$PWD/runtime/cargo-home"
export CARGO_TARGET_DIR="$PWD/runtime/target"
mkdir -p .cache/evidence/macos-native-preflight-20261021
cargo test --locked --offline -p orbit-core --test macos_native_preflight \
  native_keychain_roundtrip -- --ignored --exact --test-threads=1
```

测试会先在代码指定的目录记录新条目的引用，再尝试写入、回读、更新和删除。若进程被强制结束，按当次 journal 核对该条目的清理状态。此测试覆盖适配器往返，GUI 和 helper 的系统身份、升级连续性需分别验收。[测试实现](../crates/orbit-core/tests/macos_native_preflight.rs)

## 现有 CI

GitLab 的 validate 作业使用预置工具和 Chrome 的 `orbit-macos-arm64`、`orbit-windows-x64` 原生 runner，运行前端、浏览器、协议、Rust 和合成签名检查。通过后分别打包 macOS 应用和构建 Windows 可执行文件，Windows NSIS 是单独的手动作业。[GitLab CI 配置](../.gitlab-ci.yml)
