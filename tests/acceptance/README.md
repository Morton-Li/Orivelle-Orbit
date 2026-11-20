# 验收夹具与保留回执

方法、历史结论和未运行边界集中在[验证指南](../../docs/testing/validation.md)，构建/发布流程见[开发指南](../../docs/development/build-and-release.md)。本目录保留可执行夹具和独有机器回执。

| 资料                                                         | 用途与范围                                                               |
| ------------------------------------------------------------ | ------------------------------------------------------------------------ |
| [Panel 夹具](panel/)                                         | 原生本地验收的 Docker/HTTPS、种子与浏览器驱动；受任务目录/资源所有权约束 |
| [Windows v1.0.2](desktop/v1.0.2-windows.json)                | v1.0.1 → 候选 v1.0.2 的安装/数据保留，受控 HTTPS peer                    |
| [DMG v1.0.2](desktop/v1.0.2-dmg.json)                        | 挂载介质及 Finder 布局，不含安装后更新/重启                              |
| [macOS 快捷面板](desktop/menubar-native-2026-11-14.json)     | 真实单屏/全屏点击与失败修复；多屏暂缓，后续右键范围另见验证指南          |
| [Windows 原生客户端](desktop/windows-native-2026-11-15.json) | 托盘、Codex/Claude 实际请求与配置恢复，最终包从 current_delivery 读取    |
| [Windows updater](desktop/windows-updater-2026-11-15.json)   | 临时信任密钥下签名 NSIS 实际安装/重启；区分包构建与断言脚本来源          |
| [macOS 焦点](desktop/tray-focus-native-2026-11-16.json)      | NSPanel/WKWebView 引擎焦点，不代表完整 Tauri 或真实图标点击              |

历史 JSON 原样保留，绑定各自源码和产物；旧参数复用或界面只描述当时版本。不要改写回执匹配新产品，旧 Markdown 报告通过验证指南中的固定 Git 版本追溯。
