# 本地 Panel 授权验收

临时环境运行真实 Panel、PHP-FPM、MySQL、队列及 Scheduler。Orbit 原生 Service 走真实 HTTPS、Web 登录与明确同意、PKCE、安装证明、自动 key 交付及撤销。Vault 和客户端路径使用项目内 fixture，不启动桌面应用。

此环境不运行 Edge、Sub2api 网关或推理上游。节点记录保持离线，目录负例是非公开模型过滤；不能据此声称成员权限矩阵、推理计费、浏览器界面、候选 GUI 登录或真实客户端载入通过。

## 准备和启动

使用已准备的 Node、PHP/Composer、OpenSSL、Docker Desktop，以及任务自己的 Panel `vendor`、`node_modules` 缓存。准备脚本固定 Git 提交，先核对锁文件并拒绝越界链接，再在新快照中离线整理 Composer 依赖、构建静态资源。它不读取 Panel `.env`，也不安装全局工具。

从 Orbit 任务 worktree 执行：

```sh
node tests/acceptance/panel/prepare.mjs \
  --panel-repo ../../../Panel --cached-panel ../Panel \
  --panel-ref 01d8f67a5af6ba189c933a6f214de5ecfb8010e3 \
  --php-base docker.m.daocloud.io/library/php@sha256:ff1aaac1e7832d2cfbf5b28bc1699e41e2fbeaad1be04f4245eb98d4c081fbeb \
  --port 59443 --wrong-san-port 59444
```

准备脚本输出新建的 `run` 路径。将它设为任务变量 `orbitAcceptanceRun`，然后启动：

```sh
node tests/acceptance/panel/run.mjs setup "$orbitAcceptanceRun"
```

端口仅绑定 `127.0.0.1`；PHP 和数据库只加入内部网络，TLS 入口另有专用发布网络。Docker 客户端配置和构建缓存保存在项目内，所有资源都有独立项目名及归属标签。随机合成密码、环境文件和证书私钥位于受保护的忽略目录，不输出到报告。

初始化只接受新专属 MySQL 库及固定迁移的精确默认记录，随后建立一个合成用户、两个模型和一个离线节点。安装、桌面会话、推理 key、上游账户和消费记录均不预造；安装与 key 必须由真实授权流程产生。失败记录保留，不清库重做来隐藏失败。

## 执行和证据

先按原生构建文档使用已准备工具链及项目 Cargo 缓存。独立复核后，仅执行这个完整名称的 ignored 入口，不能批量运行其他系统集成测试：

```sh
ORBIT_PANEL_ACCEPTANCE_INPUT="$orbitAcceptanceRun/native-input.json" \
  cargo test --locked --offline -p orbit-core --lib \
  service::tests::panel_acceptance::real_local_panel_pkce_key_permissions_and_revocation \
  -- --ignored --exact --nocapture --test-threads=1
```

入口仅在 macOS 测试编译中存在。测试专用 HTTP 实例限定单一 hostname、回环解析和项目 CA，保留证书链与主机名验证，拒绝错误 CA、错误 SAN 和其他 origin。Web driver 只在内存中处理本次合成账户 Cookie，检查两次 CSRF 拒绝，并在发送回调前匹配本次实际 listener；不从网页提取推理 key。

`source-manifest.json` 记录后端提交、锁文件及静态资源摘要；`runtime-setup.json` 记录初始化结果。原生子目录的 `report.json` 仅在完整成功后写入。保留失败日志、源码摘要和具体未验收项，不将组件证据升级为真实桌面或上线验收。

结束后清理本次标签核对通过的容器、网络和数据卷，保留项目内证据及镜像缓存：

```sh
node tests/acceptance/panel/run.mjs cleanup "$orbitAcceptanceRun"
```
