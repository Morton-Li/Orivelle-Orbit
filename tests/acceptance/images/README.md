# 图片链路有限真实验收

2026-11-11，在用户批准最多三张图片后，实际使用两次请求：一次生成，一次使用生成结果的编辑。每次 `n=1`，没有自动重试，没有消耗第三张授权。

桌面临时窗口使用演示账号展示高级选项。真实请求从 Orbit 原生 `Service::send_image_once` 验收入口发送，经过 Docker 内的正式 Edge 执行代码、实际 Laravel Panel 授权及结算服务，直达 `chatgpt.com/backend-api/codex/images`。原生客户端信任一个进程内项目 CA，验证保留域名 `orbit-e2e.test` 的 HTTPS；未修改系统信任或 hosts。

原生入口通过合成会话与受绑定的合成 inference key 准备账户，不包含本次桌面登录、系统 Vault 或真实用户钱包验收。Panel 使用精确受保护的独立 SQLite 文件，账户及业务钱包为合成数据。Panel 的临时测试适配器仅在调用原有 OpenAI 适配器前，从同网络命名空间的内存 broker 获取当前 Codex 账号的 access token；数据库仍只存合成上游凭据。broker 最多发放三次，实际发放两次后移除，不执行 OAuth 刷新。

生产代码源版本为 Orbit `5f8ae5d85d898d8408a67b3590b6ae8f50460e44`、Panel `c420b919a5d4d5e40e8019201bd6806a88176b92`、Edge `8b74d79c742a7299917ae4ffc0a7ebedc144ca16`。Edge 使用正式 `cmd/orivelle-edge` 构建 Linux arm64 可执行文件，编译身份为测试 fixture 的 `9.9.9` / 四十个 `a`，不作为发布构建身份验收。使用已缓存的 PHP、Alpine 和 Node 镜像，没有在宿主机安装依赖。

| 项目                   | 生成                                                               | 编辑                                                               |
| ---------------------- | ------------------------------------------------------------------ | ------------------------------------------------------------------ |
| 请求模型别名 → 上游 ID | `reasoning-test-images` → `gpt-image-2`                            | 同左                                                               |
| 请求尺寸               | `1024x1024`                                                        | `1536x1024`                                                        |
| 请求质量 / 背景        | `low` / `opaque`                                                   | `low` / `opaque`                                                   |
| 请求格式 / 压缩率      | PNG / 未设置                                                       | JPEG / 65                                                          |
| 上游标注尺寸 / 格式    | `1536x1024` / PNG                                                  | `1536x1024` / PNG                                                  |
| 完整解码后的文件       | `1536×1024` PNG                                                    | `1536×1024` PNG                                                    |
| 保存原始字节的 SHA-256 | `e3d53041ca008c9df047e30af72ed3155cf8a71b6b1e232d7375026e40a8a132` | `39d4ea4b8b160f9ddf5f7c0fbc3cddf14540e6b24d32270125bc208c29b0c7d9` |
| 原生发送及保存入口用时 | 21.36 秒                                                           | 41.36 秒                                                           |
| 本地合成销售金额       | USD 0.01                                                           | USD 0.01                                                           |

两笔 Dispatch 均成功，`attempt_number=1`，均有唯一终态 event、一次 `http_json / response_received` 上游调用记录和一个 ordinal=1 的资源 permit。冻结快照均包含 `codex-images-static-parameters-v1`。成功事件未记录可选 `upstream_status_code`，不能把该空字段表述为已观测的 HTTP 200。

合成付款人余额从 USD 1000.00 减至 999.98，成员余额保持 USD 7.00；key 与成员累计开支均增加 USD 0.02，剩余活跃租约为零。本次模型采用 `image_count` 销售计量。没有验收 token 销售计量、官方上游成本、实际剩余额度或 MySQL 并发行为。

本次证明了有限账号条件下生成、编辑、参数合同、完整解码、原始字节保存及合成结算链路。方形尺寸及 JPEG/压缩率请求的返回存在差异，不能将选项可提交表述为上游一定兑现；响应也没有提供实际执行模型身份。没有把两次成功扩展为全部模型、数量、质量、透明背景或参数组合都已验证。

结束后移除了内存凭据容器，撤销合成验收 key，并停用 demo 图片模型。Codex 原鉴权文件的内容哈希、修改时间及权限未变化；本次目录、容器配置与服务日志检查未发现实际 token。结果页仅供回看，不再接受真实生成。

原始脱敏证据保留在本次项目工作目录 `.worktrees/orbit-image-options/.cache/live-demo/evidence/acceptance-summary.json`，页面为本机 `http://127.0.0.1:18781/demo`。原始失败记录也保留：首次原生入口在发请求前遇到弱引用导致的独占性检查失败；首次结算检查误将成功事件的可选状态码当成必填，后续组合验证改用实际记录的 transport、终态和已解码文件事实。两次问题都未增加真实上游请求。

## 原生入口

`service::tests::image_live_demo::local_docker_image_once` 默认忽略，只有明确授权真实图片预算、准备好受控 Docker demo 后才可显式运行。每次只发送一张；`generation` 和 `edit` 各有一个证据文件，发送前以 `create_new` 预留，已有文件时拒绝重放。编辑必须读取前次生成的已验证文件。成功后再次尝试相同 task 会在本地被拒绝。

入口要求 `ORBIT_IMAGE_LIVE_DEMO=1`、绝对路径 `ORBIT_IMAGE_DEMO_ROOT`、`ORBIT_IMAGE_DEMO_OPERATION=generation|edit`。根目录必须含合成 `fixture.json`、`certs/ca.pem` 与 `evidence/`；真实 token 不应写入这些文件。这个测试使用项目 Store/FakeVault 和测试专用严格 TLS 客户端，不给生产应用增加可配置 Panel origin。
