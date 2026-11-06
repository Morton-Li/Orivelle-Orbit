# Orivelle Orbit

Orbit 是面向 Orivelle 服务的桌面客户端。你可以用同一账户管理 Codex 和 Claude Desktop 的接入，在图片工作台生成、编辑和保存图片，并查看账户预算与消费。

## 开始使用

1. 在 Orbit 中发起登录，再到系统浏览器完成 Orivelle 授权。具备相应权限的账户会取得与该安装关联的推理凭据。
2. 在「接入」选择账户获准的节点和模型。核对目标客户端、文件路径和脱敏差异后确认保存，再完整退出并重新打开目标客户端。需要还原客户端配置时，选择对应的备份并预览恢复。
3. 在「图片工作台」选择生成或编辑。收到的图片保存在本地，可从历史中复制或另存为。删除本地记录只影响本地材料，消费以服务端记录为准。

接入配置受已安装客户端的版本和本地策略约束。当前自动配置适用于指定版本的 macOS 客户端。具体版本、配置生效和恢复规则见[桌面接入配置](docs/desktop-configuration.md)。

## 构建平台

| 构建目标 | 候选产物 |
| --- | --- |
| macOS Apple Silicon（arm64，配置最低 macOS 15.0） | `.app` 和 ZIP |
| Windows x64（MSVC） | `.exe`，另有 NSIS 打包步骤 |

项目使用 Tauri、Rust、Vue 3 和 TypeScript。各平台的实际验证范围见[验证指南](docs/validation.md)。

## 文档

| 主题 | 内容 |
| --- | --- |
| [构建与候选包](docs/build-candidates.md) | 固定工具链、本地开发和各平台打包 |
| [桌面接入配置](docs/desktop-configuration.md) | 写入资格、预览、保存、重启和恢复 |
| [Codex 模型目录](docs/codex-catalog.md) · [Claude Desktop 配置](docs/claude-configuration.md) | 客户端专属配置和兼容范围 |
| [架构与安全](docs/implementation.md) | 登录、凭据、原生边界和图片账务 |
| [验证指南](docs/validation.md) | 自动检查、原生实测和发布前验收 |
| [更新与恢复](docs/update-recovery.md) | 更新信任、数据保护和回退 |
