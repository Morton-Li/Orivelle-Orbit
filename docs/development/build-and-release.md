# 开发、构建与发布

本文描述当前分支的开发入口、验证选择和交付合同；历史验收结果与尚未覆盖的真实环境见[验证记录](../testing/validation.md)。脚本和工作流是实际执行入口，不根据旧报告中的版本、测试数量或任务状态推断当前候选已经通过。

## 工具与项目边界

前端采用 Vue/TypeScript/Vite，原生部分为 Tauri；Rust workspace 包含 `orbit-core`、`orbit-credential-helper` 和 `orivelle-orbit`，默认成员只有前两者。版本、命令和依赖分别由 [package.json](../../package.json)、[Cargo.toml](../../Cargo.toml)、[Rust 工具链](../../rust-toolchain.toml)及锁文件维护。

- Node 精确版本为 [.node-version](../../.node-version) 中的 `24.18.0`，npm 为 `11.16.0`，Rust/Cargo/rustfmt 为 `1.97.1`。package engines 是允许范围，原生包装脚本另检查精确版本。
- 当前原生目标为 macOS arm64（`aarch64-apple-darwin`，最低 macOS 15）和 Windows x64（`x86_64-pc-windows-msvc`）。必须使用对应原生宿主；交叉编译不作为平台验证。
- macOS 需要已有 Command Line Tools/SDK；脚本默认选 `MacOSX15.4.sdk`，可用 `ORBIT_DEVELOPER_DIR` / `ORBIT_MAC_SDK` 指向已准备环境。Windows 需要可用的 MSVC 构建环境，原生运行还需 WebView2。
- [npm 项目配置](../../.npmrc)优先使用 npmmirror，[Cargo 项目配置](../../.cargo/config.toml)使用 rsproxy。依赖按锁文件准备，不能为运行检查擅自升级版本或修改宿主配置。

[native-workflow.mjs](../../scripts/native-workflow.mjs)只使用已有工具，不安装工具链。它将 Cargo home/target、临时文件及日志放在项目 `runtime/`，npm 缓存在 `.cache/npm/`，并清理继承的构建/签名覆盖变量。`ORBIT_RUST_BIN` 可指向已有锁定 Rust 工具目录；缺少前置条件时先准备环境。

## 开发与最少验证

在已准备工具的项目目录中安装锁定的前端依赖；此命令写项目依赖目录：

```sh
npm ci
npm run dev
```

Vite 固定监听 `127.0.0.1:1420` 且端口占用即失败。网页开发只验证前端；Tauri 系统能力、凭据存储和真实客户端配置需要对应原生环境，不能由浏览器 fixture 替代。

先确定变更直接影响的入口，再选择必要检查；只有修改范围、失败或未决问题要求时扩大验证：

| 改动                   | 最少入口与边界                                                                                                                                           |
| ---------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 纯文档                 | 文件/锚点链接和 `git diff --check`；不触发原生构建或真实业务                                                                                             |
| Vue/TypeScript         | `npm run format:check`、`npm run lint`、`npm run typecheck`，加相关 `npm run test -- tests/unit/<file>.test.ts`；页面流程按需加对应 Playwright spec      |
| Rust/IPC               | 在已准备的锁定项目构建环境中运行受影响 Cargo 测试；跨 core/helper/Tauri 时使用下方 `native-workflow check`。IPC 改动同时核对命令注册、生成权限及前端调用 |
| 构建、签名、安装包脚本 | `node --test tests/build/<file>.test.mjs`；改变包内容、安装或平台行为时再构建并验证对应平台产物                                                          |
| 候选整体验证           | `npm run check`、宿主适用的构建脚本测试、`native-workflow check` 及受影响浏览器/原生验收；发布仍需相应安装包证据                                         |

常用完整入口：

```sh
npm run check
node --test tests/build/*.test.mjs
node scripts/native-workflow.mjs preflight
node scripts/native-workflow.mjs check
npm run test:e2e
```

`npm run check` 包含格式、lint、类型、Vitest、生产构建和[产物夹具排除](../../scripts/check-production-artifacts.mjs)，不包含 Playwright、Node 构建脚本测试或 Rust。`native-workflow check` 执行 Rust fmt、core/helper 的 locked tests 和 Tauri locked check，不启动原生 app，也不执行所有 ignored 外部验收。

[Playwright 配置](../../playwright.config.ts)使用已有 Chrome、单 worker 和独占 Vite 服务，前端测试以[原生 fixture](../../tests/e2e/native-fixture.ts)替换 IPC；通过不代表系统 Vault、托盘、文件对话框或真实网络已经通过。Windows 构建脚本测试采用 [prepare-native](../../.github/actions/prepare-native/action.yml)中的平台清单，不能把 POSIX 专项未运行记为通过。format 检查包含 `tests/`，验收 Markdown 也须符合格式。

## 原生候选与包

统一使用 `node scripts/native-workflow.mjs <mode>`，不另造只有 Go/Rust 主程序而缺 helper 的交付路径：

| mode             | 行为                                                                                                                  |
| ---------------- | --------------------------------------------------------------------------------------------------------------------- |
| `preflight`      | 核对宿主、精确工具与 macOS SDK，写 `runtime/logs/native-preflight.json`；不构建或启动 app                             |
| `check`          | 上述 Rust 格式、core/helper 测试和 Tauri 编译检查                                                                     |
| `signer-probe`   | 用项目内合成密钥验证锁定 CLI 的签名/验签链，不使用正式私钥或频道                                                      |
| `candidate`      | 构建 release helper，并以 `system-integration` 编译原生主程序；不打安装包                                             |
| `bundle`         | macOS 生成 `.app` 与候选 ZIP；Windows 生成 NSIS，要求预先准备完整本地 NSIS 缓存并校验插件摘要，跳过 WebView2 引导安装 |
| `github-bundle`  | 未签名候选；macOS 同 `.app`/ZIP，Windows 使用 bundler 工具下载及 WebView2 downloadBootstrapper，输出标准 NSIS 文件    |
| `github-dmg`     | macOS 候选 `.app`/DMG，保留 Finder 安装布局；不启用正式 updater 发布                                                  |
| `github-release` | GitHub-hosted macOS arm64 的正式签名及 updater 产物，要求下节完整公开/私密输入                                        |

candidate/bundle 系列在构建前后冻结 HEAD/tree，要求已提交且 tracked 工作树干净；中途源码变化即拒绝。helper 来自本项目，构建脚本核对架构并嵌入摘要，[候选清单](../../scripts/candidate-evidence.mjs)保存主程序、helper、包及来源证据。清单中的 `actual_native_integration_verified=false` 明确表示构建不等于实机验收。

macOS 候选 ZIP 位于 `runtime/candidates/`，检查 `.app` 标识、版本、最低系统、资源及 LICENSE/NOTICE/PROVENANCE，再解压回验。Windows NSIS 与正式 macOS 发布文件集中到 `runtime/github-release/`；构建日志和清单位于 `runtime/logs/`，原始运行产物不累积到 docs。

## GitHub 工作流职责

| 工作流                                                                       | 触发与结果                                                                                                                                               |
| ---------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------- |
| [build.yml](../../.github/workflows/build.yml)                               | main/develop 的 push/PR 及手动触发；先执行默认 prepare-native 检查，再构建未签名候选。默认 macOS，仓库变量 `ORBIT_WINDOWS_PACKAGING=true` 才加入 Windows |
| [windows-integration.yml](../../.github/workflows/windows-integration.yml)   | develop 的相关路径变更或手动触发；`scope=all/windows-desktop/macos-dmg` 选择回归及原生验收。macOS DMG 作业另要求非空 `candidate-tag`                     |
| [windows-runner-check.yml](../../.github/workflows/windows-runner-check.yml) | 手动或 main 上自身文件变化；仅核对 Windows x64/PowerShell 环境，不构建或验收 Orbit                                                                       |
| [macos-self-signed.yml](../../.github/workflows/macos-self-signed.yml)       | 仅 main 上手动运行，使用 codesign 环境生成自签名 DMG artifact；不创建正式 Release 或 updater 频道                                                        |
| [release.yml](../../.github/workflows/release.yml)                           | 稳定 tag push 或 main 上手动指定 tag。两平台产物通过后，只有 tag push 路径会发布；手动运行仅验证和上传 artifacts                                         |

prepare-native 在临时 hosted runner 准备锁定工具/依赖，默认执行前端、宿主构建脚本及原生检查。集成工作流的包作业可设置 `validate=false`，不能由“包成功”反推所有回归已跑。只有验收脚本/指定工作流改动时，集成入口才允许在同仓库祖先关系、应用源码未变和摘要匹配后复用旧 artifacts；保留实际 build source/run，不能称其为当前源码重建。

## 安装包证据

- Windows 发布阶段用 7-Zip 检查 NSIS 完整性、提取主程序，核对 ProductVersion 与 x64 PE。[windows-package-evidence.mjs](../../scripts/windows-package-evidence.mjs)仅允许固定 Tauri 的 `UNK` → `NSS` 包类型标记变化，其余字节必须与构建主程序相同。
- Windows 原生集成沿 [prepare-installers.ps1](../../tests/windows/prepare-installers.ps1)与 [with-panel-fixture.ps1](../../tests/windows/with-panel-fixture.ps1)检查正式基线校验和、安装/升级及真实系统行为。Panel/图片服务为受控 HTTPS fixture，不能据此证明真实上游或第三方客户端配置。
- macOS [verify-dmg-layout.mjs](../../scripts/verify-dmg-layout.mjs)只在 GitHub-hosted Mac 运行：实际校验并只读挂载 DMG，核对 app、Applications 链接、背景精确字节、Finder 布局及截图。证据写 `runtime/logs/dmg/`；这不包含用户机器实际拖拽安装或在线更新重启。

每次记录候选来源、产物摘要、实际执行组件、失败及未运行项目；[历史验证](../testing/validation.md)中的旧截图或统计不能代替本次结果。

## 正式发布与 updater 合同

[github-release.mjs](../../scripts/github-release.mjs)只接受 `vMAJOR.MINOR.PATCH`，不接受预发布/构建后缀。source 作业检查标签格式及 checkout 的 main 祖先关系；两个平台按同一 source commit 构建。`prepare-source` 只允许 hosted checkout，按 tag 改 package/lock、Tauri 与 Cargo 版本，必要时生成临时构建提交；Release 仍指向原 tag，构建清单记录派生来源，不在本地手动复刻该提交。

codesign 环境所需公开变量和 secrets：

| 输入                                                                | 用途                                                                                     |
| ------------------------------------------------------------------- | ---------------------------------------------------------------------------------------- |
| `ORBIT_RELEASE_UPDATER_PUBLIC_KEY`                                  | 仓库变量，编入 app/updater 配置并用于发布前验签                                          |
| `TAURI_SIGNING_PRIVATE_KEY` / `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`  | updater 签名 secrets，仅传给构建完成后的 bundler                                         |
| `CODESIGN_CERTIFICATE_P12_BASE64` / `CODESIGN_CERTIFICATE_PASSWORD` | macOS 签名 secrets，须匹配[固定证书指纹](../../scripts/macos-signing-certificate.sha256) |

签名公开配置还包括由仓库身份派生的 GitHub latest.json 地址和固定 macOS 证书摘要；基础 Tauri 配置为空频道/占位公钥。普通候选不会继承正式签名环境。代码签名由 [package-self-signed-macos.sh](../../scripts/package-self-signed-macos.sh)在 hosted Mac 临时 Keychain 内完成并清理，核对证书有效期、私钥及嵌入证书；自签名不表示 Apple 公证已完成。

正式文件为 `Orivelle-Orbit-v<版本>-macos-arm64.dmg`、`-windows-x64-setup.exe`、macOS `.app.tar.gz` 与 `.sig`。发布脚本验签后生成 `latest.json` 和 `SHA256SUMS`，只接受完整约定集合；检查 tag 未移动、版本高于现有稳定版，再上传并核对 draft 的远端大小/SHA-256，最终公开。已公开同 tag 或冲突 draft 拒绝，不自动替换稳定版。

updater 只发布 `darwin-aarch64`，Windows 仍手动下载安装包。签名同时绑定包内容及可信目标版本，客户端保留 `requireSignedVersion=true` 和禁止降级；应用代码签名、updater 签名、包 SHA-256 是分别核对的事实。源码/合成 signer 测试与实际安装、数据保留、失败恢复和重启验收分别记录，触发工作流、推 tag 或公开 Release 按本次明确发布范围执行。
