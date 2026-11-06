# 本地 Panel 授权验收

这项可选集成测试在本机 Docker 中运行 Panel、PHP-FPM、MySQL、队列与调度器。Orbit Service 经隔离 HTTPS 入口验证网页登录、同意授权、PKCE、安装证明、安装关联 key 的交付和撤销。Web 驱动使用合成账户，Vault 使用内存实现，客户端路径使用测试夹具。节点保持离线，证据范围限于 Panel 授权和 API 合同。

## 准备

在 macOS 上准备 Node、满足 Panel Composer 平台要求的 PHP 和 Composer、支持 `req -addext` 的 OpenSSL、Command Line Tools，以及本地 Docker Desktop。运行脚本使用 `/usr/local/bin/docker`、`desktop-linux` 的本地 Unix socket 与 Docker Desktop 自带的 Compose 插件。

需要能访问对应的 Panel 源码与已准备的 `vendor/`、`node_modules/`。设置以下参数后，从 Orbit 仓库根目录运行命令。

- `panel_repo` 指向含桌面授权接口及相应迁移的 Panel 仓库，`panel_ref` 为选用的完整 40 位小写 Git 提交。
- `cached_panel` 指向准备好的 Panel 依赖目录。该目录须同时包含与 `panel_ref` 一致的 `composer.lock`、`package-lock.json`。
- `php_base` 使用满足 Panel 运行要求、提供 `docker-php-ext-install` 的 PHP-FPM 基础镜像，按 `镜像名@sha256:摘要` 固定。MySQL 和 Caddy 镜像由 Compose 配置固定。[Compose 文件](compose.yml)
- 两个端口均需空闲，并可改为不同的 1024 至 65535 端口。

```sh
node tests/acceptance/panel/prepare.mjs \
  --panel-repo "$panel_repo" --cached-panel "$cached_panel" \
  --panel-ref "$panel_ref" --php-base "$php_base" \
  --port 59443 --wrong-san-port 59444
```

脚本归档指定提交，核对锁文件和依赖链接，在项目内新快照中离线整理 Composer 依赖并构建静态资源。环境文件、合成凭据和测试证书独立生成，保存在忽略目录 `.cache/panel-acceptance/`。其中的输入文件、私钥和原始日志只供本次本地测试使用。

## 执行

将准备脚本输出的绝对 `run` 路径赋给 `orbit_acceptance_run`，启动独立环境。

```sh
node tests/acceptance/panel/run.mjs setup "$orbit_acceptance_run"
```

容器端口只发布到 `127.0.0.1`。初始化在本次专用 MySQL 库中执行固定迁移，创建合成用户、模型和离线节点，安装与 key 由授权流程产生。

按[原生开发指南](../../../docs/build-candidates.md)准备工具链和项目缓存，再运行 macOS 上的这一条 `ignored` 用例。

```sh
export CARGO_HOME="$PWD/runtime/cargo-home"
export CARGO_TARGET_DIR="$PWD/runtime/target"
ORBIT_PANEL_ACCEPTANCE_INPUT="$orbit_acceptance_run/native-input.json" \
  cargo test --locked --offline -p orbit-core --lib \
  service::tests::panel_acceptance::real_local_panel_pkce_key_permissions_and_revocation \
  -- --ignored --exact --nocapture --test-threads=1
```

测试仅把隔离 hostname 解析到回环地址，使用本次的 CA，并校验证书链、主机名和 origin。用例同时检查错误 CA、错误 SAN 与 CSRF 拒绝。`source-manifest.json` 记录后端提交、锁文件与静态资源摘要，`runtime-setup.json` 记录初始化结果。原生子目录中的 `report.json` 只在用例完整成功后写入。

结束后用同一运行目录清理本次容器、网络和数据卷。脚本按 Compose 项目名与归属标签核对范围，证据和镜像仍可用于复核。

```sh
node tests/acceptance/panel/run.mjs cleanup "$orbit_acceptance_run"
```
