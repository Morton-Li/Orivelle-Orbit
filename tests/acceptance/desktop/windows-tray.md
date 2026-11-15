# Windows 托盘、启动与客户端配置验收

当前行为与 2026-11-15 验收绑定 `feature/windows-client-configuration`。机器可读的源码、安装包与客户端回执见 [Windows 验收记录](windows-native-2026-11-15.json)。历史初版托盘面板来自 `fd6b50e`，控制台与左右键修复来自 `4d83691`，通过 `a20423a` 合入 develop。

## 当前行为

- Windows release 主程序和 credential helper 都使用 x64 GUI 子系统。后台探测同时使用 `CREATE_NO_WINDOW` 与隐藏 PowerShell；打开浏览器使用 Windows ShellExecute。
- 原生存储、helper 部署和客户端探测在后台初始化，bootstrap 等待就绪后执行后台 I/O，避免阻塞窗口消息。初始化期间的关闭请求仍按默认行为收起。
- Windows 与 macOS 左右键都展开或收起同一只读快捷面板。创建成功后解除托盘原生菜单绑定；创建失败保留备用入口。设置及受任务保护的退出在面板“更多操作”中。
- 面板读取共享账户、用量、接入和任务状态，跟随主题与语言；收起停止轮询，保留服务。Windows 双击唤回主窗口；失焦、Esc、关闭面板与导航收起面板。
- Windows 面板使用紧凑圆角、Segoe UI、无菜单栏尖角，不新增任务栏窗口。按点击图标所在显示器的工作区和物理像素定位，并保留展开锚点以适应隐藏图标区关闭。
- 面板的受限权限不能提交图片、配置客户端或修改设置。

## 客户端配置与恢复

Windows x64 已接入原生配置事务。MSIX 核对注册身份；传统 EXE 核对官方签名、安装布局和当前客户端配置能力。Claude 另外核对签名 PE 中绑定的 ASAR 头与相关成员摘要。版本不是精确白名单，但升级会使旧预览失效；不认识的能力继续拒绝写入。

配置保存与恢复重新核对当前安装、文件句柄、文件 ID、用户 SID、DACL、预览与原文件摘要。拒绝重解析点、UNC、ADS 和不受信的写权限。Windows Credential Manager 按用户存储凭据；这不构成同一用户内的应用隔离。

Claude 首次运行会在主设置中新增 preferences、coworkUserFilesPath 等内容。严格恢复仍拒绝原文件发生变化；明确选择“保留 Claude 设置后切回”时，仅恢复 Orbit 管理的 deploymentMode，并精确恢复原 profile/index，保留当前其他原始设置值。该模式也覆盖原主设置文件不存在的情况，继续检查预览后变更并撤销当前 helper 授权。

本次顺序为：

1. Claude MSIX `5.11492.5.0`：预览、保存、实际 GUI 发送消息、helper 与本地映射、受控 HTTPS Messages 回复、恢复及授权撤销通过。
2. 完整卸载 Claude MSIX，删除并检查已知 Claude 专属应用数据目录；保留 Codex、Orbit 和其他用户数据。
3. 普通用户安装 Anthropic 有效签名的完整离线 Squirrel EXE `5.11492.5`，安装退出码 0、MSIX 不再注册；重复同一实际 GUI 与恢复验收通过。
4. Codex MSIX `26.1111.2228.0`，内置 CLI `0.169.0-alpha.11.1`：实际 CLI 读取 Orbit 生成目录，并完成 helper 认证的 Responses 请求，随后精确恢复 config/catalog 并验证旧绑定失效。Codex 官方离线桌面包采用 MSIX；没有把 CLI 安装器当作桌面 EXE 验收。

请求经过真实安装的客户端、Windows 凭据存储、真实 helper 和配置文件，远端为专用受控 HTTPS peer，没有调用计费上游。

最终包补验还保留了一次 Codex 恢复提交被安装校验拒绝的记录。当时已进入 publishing，未把中断写成成功。随后只读核对安装版本未变、两个签名均有效；重启应用后，原生 journal 续恢复将原配置写回并把源事务标为 restored，旧 helper 确认拒绝凭据。此次中断的具体触发条件未确定，原始失败与恢复回执分别保存；后续最终复验结果单列，不能用恢复通过抹掉首次中断。

## 原生桌面与自动化

原生验收使用最终 NSIS 安装后的程序，而非浏览器 UA 或交叉编译结果。回执绑定生产文件清单和安装包/可执行文件 SHA-256；包内程序只允许固定 Tauri 的 UNK→NSS 包类型标记变化。

托盘脚本限定可见通知区的 `NotifyItemIcon`，按实际名称查找 Orbit，等隐藏区展开动画后的边界稳定再发送真实鼠标事件。不能使用隐藏区旧坐标，也不能把同名普通任务栏按钮当作通知图标。是否新增任务栏窗口以真实 Shell 条目为准；Tao 使用 `ITaskbarList::DeleteTab`，不以 `WS_EX_TOOLWINDOW` 标记作替代断言。

启动检查记录窗口消息响应及本进程树的可见控制台；面板检查覆盖左右键、浅深主题、前台焦点、工作区、Esc、失焦、导航和 IPC 权限。其余图片、文件对话框、剪贴板、后台任务和退出保护结果以机器回执逐项为准。

失败采集保留在项目 `runtime/windows-vm/evidence/` 与 `runtime/logs/`：最初托盘脚本命中 OneDrive 通知；旧选择器未考虑节点延迟文字；旧 HTTPS 夹具遗漏面板用量汇总路由。修正验收脚本和夹具后重新执行，未将这些失败记录改写为通过。

## 边界

本次使用单台 Windows x64 VM 的单屏环境及已有 WebView2。真实多屏、不同 DPI、全新系统的 WebView2 引导下载不在本次通过范围。macOS 同一左右键修复已完成代码与隔离原生检查，但本次新右键的真实状态栏鼠标验收仍未通过自动化；详见 [macOS 记录](menubar.md#2026-11-14-左右键一致性修复)。

本次为版本元数据 `0.1.0` 的未签名内部候选，未推送、打 tag 或发布。正式版本号和发布签名继续走既有发布流程。默认启动检查更新已设为开启，保留已有保存的用户选择；Windows 现行更新能力仍为手动安装包。

## 2026-11-14 控制台与右键修复

历史分支 `fix/desktop-console-and-tray-clicks` 完成 GUI 子系统、隐藏子进程、左右键同一面板和 NSIS PE 检查。初版“右键保留原生菜单”已经废止。此前只有 macOS 编译/隔离检查，不能把当时记录当作 Windows 原生通过；当前 Windows 结果以本文及对应日期回执为准。
