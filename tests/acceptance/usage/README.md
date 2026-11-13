# 用量页候选验收记录

2026-11-13，本地 macOS arm64。界面样式待用户确认，候选尚未合并 develop。

## 功能与分支

- Orbit：`feature/usage-page`。基于已合并价格区移除与配置兼容改动的 develop `261f455bb1c950510c42a21bd5d6c8c5a30f05c3`。
- Panel：`feature/orbit-usage`，候选 `47db1d8336721755c70a1d080bcb00ec78571901`。隔离工作区 `.worktrees/orbit-usage/Panel`；原 Panel develop 保持不变。
- 账户全部入口的时段汇总、每日请求／已确认消费趋势、调用记录结果筛选、签名游标分页和请求详情。只读账户无需推理 key 或节点即可查询本人数据。
- 不同币种分开；金额保留十进制字符串，待确认金额不补为零；缓存与推理 Token 不重复加入总数。汇总与明细各自沿用平台原有时间及结果语义。

## 检查结果

| 范围 | 结果 |
| --- | --- |
| 前端格式、lint、类型、生产构建及演示数据排除 | 通过 |
| 前端单元 | 163 通过，含跨账户迟到响应、只读、错误保留、日期边界、大金额及多币种 |
| 浏览器工作流 | 29 通过，含用量筛选、分页、详情、浅／深色及窄窗口 |
| Rust 核心 | 278 通过；3 项原有显式外部验收保持 ignored |
| macOS 原生 | typed IPC 检查与隔离 `.app` 构建通过 |
| Panel 相关 HTTP 回归 | 76 通过、923 断言；2 项原有测试 skipped |
| Panel PHPStan 与变更 PHP 格式 | 通过 |

前端／浏览器／原生证据在 Orbit `.cache/usage-*.log`、`.cache/usage-demo/`；Panel 证据在隔离工作区 `storage/framework/testing/orbit-usage/`。后端使用独立内存 SQLite 与合成应用 key，不连接现有业务数据库。

## 界面演示

独立应用 `target/debug/bundle/macos/Orbit Usage Preview.app`，标识 `io.orivelle.orbit.usage-preview`。使用单独的项目内数据目录与非持久化 WebView，页面明确标记合成数据。演示构建的独立入口／bridge 在 `.cache/usage-demo/`；普通生产构建不包含这些演示模块。桌面窗口已直接显示用量页，供用户确认样式。

演示未写入已安装桌面工具的配置，没有真实模型调用、实际消费或生产部署。真实平台的汇总功能需要包含新 `/api/v1/orbit/usage/summary` 的 Panel 版本；本轮未执行真实平台联调或 MySQL 验收。
