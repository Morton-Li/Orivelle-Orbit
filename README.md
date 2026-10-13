# Orivelle Orbit

独立桌面客户端，优先管理 Codex 与 Claude Desktop 的 Orivelle 接入；同时提供本地图片生成、编辑和历史工作台。四个主要页面为首页、图片工作台、接入、设置。通过系统浏览器完成 Orivelle 授权，API key 不作为桌面登录身份。

当前为项目内开发候选。目标平台为 **Windows 11 25H2 x64**、**macOS 15+ Apple Silicon**；平台支持必须以真实系统验收为准。macOS Intel 已移除。Linux、Windows ARM64和移动端不在范围内。

开发使用已经准备好的 Node 24.18.0、npm 11.16.0、Rust 1.97.1，以及相应平台构建工具。版本和依赖分别固定在 `.node-version`、`rust-toolchain.toml`、`package-lock.json`、`Cargo.lock`。不使用动态 latest。

```sh
mkdir -p .cache/npm .cache/tmp runtime/cargo-home runtime/target
TMPDIR="$PWD/.cache/tmp" npm ci --cache "$PWD/.cache/npm" --registry https://registry.npmmirror.com
TMPDIR="$PWD/.cache/tmp" npm run check
TMPDIR="$PWD/.cache/tmp" npm run test:e2e
```

npm 镜像缺少版本或响应异常时，可显式使用官方 registry，缓存仍保留在项目内。浏览器测试使用已安装的 Chrome，不自动安装浏览器或系统工具。Rust 构建也应设置项目内 `CARGO_HOME`、`CARGO_TARGET_DIR` 和临时目录；缺少工具链时先准备前置条件。

普通 debug 构建采用项目内数据和测试配置根，并拒绝真实系统安全存储。真实客户端配置、原生安全存储以及实际桌面接入测试需专用环境授权。浏览器和 fake gateway 测试分别是界面、合同证据，不能替代客户端或平台验收。

配置应用按客户端独立预览、确认和恢复。保存后显示待重启/载入未确认；只有匹配配置、应用版本和连接上下文的实际证据才能确认载入或连接。Claude configLibrary 的完整 profile/metadata 格式仍须专用环境导出验证，不能猜测覆盖。

图片请求复用 Edge 严格 JSON 合同。异常后不自动重发；已收到的本地保存材料可以仅重试本地保存。没有服务器图片结果恢复、生成去重、幂等或成功结果重放。执行记录和结算状态以 Panel 为权威，金额以十进制字符串展示。

详见 [实施与安全合同](docs/implementation.md)、[验证记录与环境门禁](docs/validation.md)。正式签名、发布、公钥与更新渠道配置另行授权。
