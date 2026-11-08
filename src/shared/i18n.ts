import { ref } from "vue";
import type { Locale } from "./contracts";

export const locale = ref<Locale>("zh-CN");
const zh = {
  "nav.home": "首页",
  "nav.images": "图片工作台",
  "nav.access": "接入",
  "nav.settings": "设置",
  "nav.label": "主要导航",
  "app.subtitle": "连接你的 AI 工作流",
  "app.synthetic": "合成测试数据 · 不代表真实接入或结算",
  "action.login": "连接账户",
  "action.cancel": "取消",
  "action.close": "关闭",
  "action.continue": "继续",
  "action.preview": "预览配置",
  "action.apply": "确认应用",
  "action.restore": "预览恢复",
  "action.logout": "退出登录",
  "action.save": "保存",
  "action.copy": "复制图片",
  "action.edit": "再次编辑",
  "action.delete": "删除本地历史",
  "action.retrySave": "重试本地保存",
  "action.refresh": "刷新",
  "action.manage": "管理接入",
  "action.more": "更多操作",
  "login.title": "连接 Orivelle",
  "login.description":
    "在系统浏览器登录并授权 Orbit，接入凭据将由授权流程安全取得。",
  "login.panel": "Panel 地址",
  "login.service": "Orivelle 官方服务",
  "login.officialOnly": "此版本仅连接 Orivelle 官方服务，请重新登录。",
  "login.hint": "继续后将在系统浏览器打开 Orivelle 登录与授权页面。",
  "login.waiting": "请在系统浏览器完成登录和授权。",
  "home.eyebrow": "桌面接入",
  "home.title": "让 AI 工具接入 Orivelle",
  "home.description": "从一个账户管理桌面工具的节点、模型与接入配置。",
  "home.account": "你的账户",
  "home.loggedOut": "连接账户后开始",
  "home.loggedOutHint":
    "复用 Orivelle 登录、验证和账户权限，无需手动输入 API key。",
  "home.wallet": "钱包余额",
  "home.budget": "当前可用预算",
  "home.updatedAt": "账户数据更新于",
  "login.refreshing": "正在更新会话",
  "login.connected": "账户连接已完成。",
  "login.denied": "浏览器授权已拒绝，当前账户保持原状态。",
  "login.expired": "浏览器授权已过期，请重新连接。",
  "login.cancelled": "已取消本次浏览器授权。",
  "common.updateMaintenance":
    "正在准备安装更新，新的图片请求与配置变更已暂停。",
  "home.balanceUnknown": "额度暂不可确认",
  "home.accountReady": "账户已连接",
  "home.permissionDenied": "当前账户无自动取得接入凭据的权限",
  "home.codex": "Codex 桌面",
  "home.claude": "Claude Desktop",
  "home.codexHint": "Responses 接入 · 用户配置可能与 CLI / IDE 共享",
  "home.claudeHint": "第三方推理配置影响整个桌面应用：Chat、Code 与 Cowork",
  "home.images": "也可以从图片开始",
  "home.imagesHint": "生成、编辑和管理本地结果，所有任务使用获准的连接。",
  "client.pending": "待检测",
  "client.notInstalled": "未检测到应用",
  "client.notConfigured": "尚未配置",
  "client.saved": "配置已保存",
  "client.restart": "待重启 / 载入未确认",
  "client.loaded": "客户端载入已确认",
  "client.verified": "连接验证通过",
  "client.notTested": "连接未验证",
  "client.failed": "连接验证失败",
  "client.conflict": "配置冲突",
  "client.unsupported": "版本或配置格式待验证",
  "client.managed": "组织管理配置",
  "client.configurationAllowed": "可预览本地用户配置",
  "client.configurationBlocked": "本地用户配置写入受阻",
  "client.configurationUnverified": "本地用户配置写入资格待确认",
  "client.remotePolicyUnknown": "远端组织策略未确认，可能影响配置生效。",
  "access.eyebrow": "接入管理",
  "access.title": "选择工具，确认每次切换",
  "access.description":
    "先查看账户、节点、模型和影响范围，再分别应用到目标桌面客户端。",
  "access.connection": "当前连接",
  "access.node": "推理节点",
  "access.model": "接入模型",
  "access.selectModel": "选择获准的模型",
  "access.noModels": "连接账户和节点后取得模型",
  "access.noConnection": "尚无账户连接",
  "access.noNode": "尚无可用节点",
  "access.selectNode": "选择获准的节点",
  "access.nodeUnavailable": "节点当前不可用",
  "access.readonly":
    "只读账户可查看预算和消费，自动取得接入凭据需现有业务写权限。",
  "access.previewTitle": "确认配置与影响范围",
  "access.previewHint":
    "只应用本次预览，客户端需要完整重启。保存配置不会自动发送推理请求。",
  "access.paths": "目标文件",
  "access.impact": "影响范围",
  "access.changes": "脱敏配置差异",
  "access.backups": "配置备份",
  "access.noBackups": "应用配置后会创建加密的完整操作组备份。",
  "access.backupHint":
    "保留 30 天，每个客户端最多 5 组。第三方修改会阻止自动覆盖恢复。",
  "access.backupAccount": "备份所属账户",
  "access.selectBackupAccount": "选择明确的原账户与 Panel 地址",
  "access.saved":
    "配置已保存。请完整退出目标应用并重新打开，载入和连接仍待确认。",
  "access.restoreLocalConfiguration": "确认恢复配置",
  "access.preserveClaudePreferences": "保留偏好后预览恢复",
  "access.preferencesRestoreHint":
    "仅协调 Claude 主配置中的偏好变化；其他字段或配置库变化仍会阻止恢复。",
  "access.restorePreservingPreferences": "确认保留偏好并恢复接入",
  "access.restoredPreservingPreferences":
    "接入配置已恢复，后续偏好已保留。请完整退出目标应用并重新打开，载入和连接仍待确认。",
  "access.restored":
    "原配置已恢复。请完整退出目标应用并重新打开，载入和连接仍待确认。",
  "access.exportImport": "确认导出 Claude 诊断候选",
  "access.importHint":
    "仅保存供核查的 Claude 配置诊断候选，不会写入客户端配置，也不代表可直接导入。客户端的保存、载入和连接状态均不会因此改变。",
  "access.importExported":
    "Claude 配置诊断候选已导出；客户端配置、载入和连接状态未改变。",
  "access.previewExpired":
    "预览已失效，或账户、节点、凭据、模型、组织策略及客户端版本已变化，请重新预览。",
  "access.restorePreviewExpired":
    "恢复预览已失效，或所选备份已删除、到期或发生变化，请重新预览恢复。",
  "access.policyUnverified":
    "尚未确认目标应用的本地配置写入资格或组织管理限制，暂时无法变更或导出配置。请重新检测并预览。",
  "access.policyChanged":
    "目标应用的本地配置写入资格或组织管理策略已变化，本次确认已失效。请重新检测并预览配置。",
  "access.configurationConflict":
    "检测到目标配置或操作状态已变化，已停止覆盖。请核对变更后重新预览。",
  "access.claudeMappingUnavailable":
    "Claude 本地模型映射不可用。请保持 Orbit 运行，并检查本地端口是否被占用，核对后重新预览。",
  "access.saveLocalConfiguration": "确认保存本地配置",
  "access.claudeConfigurationTarget": "已安装 Claude Desktop 的配置目标",
  "access.claudeDesktopScope":
    "第三方推理配置影响 Claude 整个桌面应用，包括 Chat、Code 与 Cowork。仅变更下列目标文件，保存后需完整重启；保存不证明客户端已载入或连接。",
  "access.claudeConfigurationUnavailable":
    "尚未确认当前 Claude Desktop 版本的配置合同，暂时无法保存或恢复。诊断候选导出不代表配置已生效。",
  "access.claudeVerificationTimeout":
    "Claude 应用身份校验超时，操作已停止。请核对状态后重新预览。",
  "access.claudeNativeMigrationRequired":
    "Claude 需要先迁移现有本地配置。请核对当前操作状态，完成 Claude 原生迁移并退出后重新预览。",
  "access.claudeConfigurationChanged":
    "Claude Desktop 配置目标或本地写入资格已变化，本次确认已失效。请重新预览。",
  "access.claudeRestoreContextUnavailable":
    "此 Claude 备份缺少可核验的恢复上下文，暂时无法自动恢复。请核对原配置。",
  "access.claudeContextChanged":
    "Claude 配置的账户、节点、授权或文件状态已变化，本次确认已失效。请重新预览。",
  "access.claudeSourceUnsupported":
    "现有 Claude 配置来源无法安全迁移。自动配置操作已停止，请核对当前配置来源并处理恢复提示。",
  "access.codexConfigurationTarget": "已安装 Codex 的配置目标",
  "access.codexConfigurationCli": "已安装目标的配置 CLI 版本",
  "access.codexDefaultScope":
    "本次仅变更下列默认用户配置文件，可能同时影响使用同一目录的 CLI / IDE。远端、云端及自定义配置目录不在本次范围。",
  "access.remotePolicyWarning":
    "远端组织策略尚未确认，可能影响本地配置生效。确认后仅变更默认用户配置，载入与连接仍待验证。",
  "access.codexConfigurationUnavailable":
    "尚未确认已安装 Codex 的配置合同或默认用户配置路径，本次不能保存或恢复。请重新预览。",
  "access.codexConfigurationChanged":
    "已安装 Codex 的配置目标或本地写入资格已变化，本次确认已失效。请重新预览。",
  "access.codexCatalog": "Codex 模型目录",
  "access.codexMenuAliases": "模型菜单调用名称（本次目录）",
  "access.codexDefaultAlias": "默认模型调用名称",
  "access.codexDefaultEffort": "默认推理档位",
  "access.codexDeclaredCli": "声明目标 CLI（须匹配已安装配置目标）",
  "access.codexRuntime": "本机接入来源核验",
  "access.codexRuntimeCli": "本机接入 CLI 版本",
  "access.codexRuntimeVerified": "本机接入：本次预览的执行来源已核验",
  "access.codexRuntimeUnverified": "待核验",
  "access.codexRuntimeBlocked":
    "本机运行来源仍待核验。保存本地配置不代表该运行实例已使用此配置。",
  "access.codexRuntimeUnavailable":
    "本机接入：尚未核验目标桌面应用的执行来源，请重新预览并核对本次操作是否可用。",
  "access.codexRuntimeChanged":
    "本机接入：目标桌面应用的执行来源已变化，本次确认已失效。请重新预览配置。",
  "access.codexRestoreContextUnavailable":
    "此备份缺少可核验的恢复上下文，无法自动恢复。请手动核对原配置。",
  "access.codexOverrideUnavailable":
    "现有显式设置可能覆盖本次配置，暂时不能自动应用。请先手动核对后重新预览。",
  "access.codexEvidenceBoundary":
    "已安装版本的配置合同、模板声明和目录生成不代表客户端已载入或连接通过。保存配置后仍需重启并分别确认。",
  "access.codexBindingUnavailable":
    "当前模型缺少可用的 Codex 模板声明。请刷新模型列表，或联系管理员补充声明后重新预览。",
  "access.codexReasoningUnavailable":
    "当前推理档位策略不足以生成完整目录。请刷新模型权限后重新预览。",
  "access.codexCatalogChanged":
    "模型目录、权限或连接状态已变化，本次预览已失效。请刷新后重新预览。",
  "access.codexCatalogUnavailable":
    "当前模型目录无法安全生成。请核对模板声明、版本及推理档位后重新预览。",
  "access.removeHint":
    "删除连接会清除此连接的本地会话和推理凭据，保留本地历史、安装证明与备份恢复密钥。服务器推理 key 仅在你另行选择撤销时撤销。",
  "access.revoke": "撤销本安装推理 key",
  "access.remove": "删除本地连接",
  "access.revokeHint":
    "影响此安装的桌面工具和图片调用；其他 key 保持原状态，已接收的请求可能继续结算。",
  "images.eyebrow": "本地图片工作台",
  "images.title": "把想法变成图像",
  "images.description": "选择获准模型，输入提示词，或加入参考图进行编辑。",
  "images.create": "创作",
  "images.history": "本地历史",
  "images.prompt": "提示词",
  "images.promptPlaceholder":
    "描述主体、构图、光线与风格。添加参考图后将使用编辑接口。",
  "images.inputs": "参考图",
  "images.addInputs": "添加参考图",
  "images.drop": "拖入 PNG、JPEG 或 WebP，也可从文件选择器添加",
  "images.noInputs": "未添加参考图",
  "images.params": "生成参数",
  "images.count": "结果数量",
  "images.quality": "质量",
  "images.size": "请求尺寸",
  "images.background": "背景",
  "images.default": "使用服务默认值",
  "images.generate": "生成图片",
  "images.edit": "编辑图片",
  "images.ready": "准备好下一张图片",
  "images.empty": "结果将在这里预览",
  "images.emptyHint": "没有服务端阶段事件时，只显示等待响应与经过时间。",
  "images.noHistory": "本地历史还没有图片",
  "images.noHistoryHint":
    "收到的图片会保存在本地；删除历史不会改变服务器账务。",
  "images.waiting": "等待响应",
  "images.saving": "保存本地结果",
  "images.saveFailed": "本地保存失败",
  "images.unconfirmed": "结果待确认",
  "images.failed": "请求失败",
  "images.cancelled": "本地已取消",
  "images.completed": "本地结果已保存",
  "images.cancelHint": "取消本地等待不能保证服务端停止或不结算。",
  "images.promptLimit": "提示词超过 32768 个 UTF-8 字节。",
  "images.requestedModel": "请求模型",
  "images.actualUnknown": "实际型号未观测",
  "images.unverified": "此模型的参数组合尚未验证",
  "images.settlement": "结算状态",
  "images.pendingCharge": "费用待确认",
  "images.reconciliation": "待核账",
  "images.charged": "已结算",
  "images.recorded": "已记录费用分项",
  "images.unbilled": "服务端确认未计费",
  "images.incomplete": "记录不完整",
  "images.priceSales": "平台销售价",
  "images.priceOfficial": "官方参考价",
  "images.priceEstimate": "运营估价",
  "images.priceConfiguredReference": "平台配置参考价",
  "images.pricePartial": "仅部分计费项配置了参考价，缺失价格保持未知。",
  "images.legacyBilling": "旧按张计费口径",
  "images.tokenBilling": "按图片 token 合同计费",
  "images.removeInput": "移除参考图",
  "images.zoom": "放大预览",
  "images.deleteHint": "只删除所选本地图片与历史，不改变服务器记录和费用。",
  "images.inputLimit": "参考图数量超过当前模型上限，请先移除部分参考图。",
  "images.reuse": "复用参数",
  "images.execution": "服务端执行记录",
  "images.executionConfirmed": "执行事实已确认",
  "images.chargeItem": "费用分项",
  "images.pricing": "价格与计费口径",
  "images.unpricedFree": "平台确认无配置价格，按现有免费准入规则处理",
  "images.priceHint":
    "价格按服务端单位显示；预算可能采用软准入，实际结算以 Panel 记录为准。缓存输入属于输入子集，不重复累计。",
  "meter.input": "输入 token",
  "meter.cachedInput": "缓存输入 token",
  "meter.cacheWrite": "缓存写入 token",
  "meter.output": "输出 token",
  "meter.reasoning": "推理 token",
  "meter.request": "请求",
  "meter.image": "图片张数",
  "meter.textInput": "文本输入 token",
  "meter.cachedText": "缓存文本输入 token",
  "meter.imageInput": "图片输入 token",
  "meter.cachedImage": "缓存图片输入 token",
  "meter.imageOutput": "图片输出 token",
  "settings.eyebrow": "偏好与安全",
  "settings.title": "让 Orbit 适合你的工作方式",
  "settings.description": "外观、文件、凭据保护与更新都在这里管理。",
  "settings.appearance": "外观与语言",
  "settings.theme": "主题",
  "settings.light": "浅色",
  "settings.dark": "深色",
  "settings.system": "跟随系统",
  "settings.language": "语言",
  "settings.files": "文件与磁盘",
  "settings.disk": "图片磁盘上限",
  "settings.cache": "预览缓存上限",
  "settings.backups": "加密配置备份",
  "settings.security": "安全存储",
  "settings.securityHint":
    "账户会话、推理凭据、安装身份和备份密钥分别保存在原生系统安全存储。拒绝访问时操作会明确失败。",
  "settings.installation": "安装所属 Panel",
  "settings.logoutHint":
    "退出清除账户和推理凭据，保留安装证明和备份恢复密钥。目标应用可能缓存凭据。",
  "settings.forget": "忘记此安装",
  "settings.forgetTitle": "清除安装身份与恢复密钥",
  "settings.forgetHint":
    "这会失去原安装证明并使相关旧备份不可解密。服务器 key 不会因此自动撤销。",
  "settings.ackLoss": "我理解相关旧备份将无法恢复",
  "settings.window": "窗口行为",
  "settings.close": "关闭窗口时",
  "settings.platformDefault": "遵循系统习惯",
  "settings.keepRunning": "保留进程继续当前任务",
  "settings.quit": "退出应用",
  "settings.quitHint":
    "真正退出与崩溃后不承诺继续生成，未确认的任务会保留待确认状态。",
  "settings.updates": "更新",
  "settings.autoCheck": "启动时检查更新",
  "settings.check": "检查更新",
  "settings.download": "下载并验证",
  "settings.install": "安装并重启",
  "settings.disabledUpdate": "发布渠道尚未配置",
  "settings.upToDate": "当前已是最新版本",
  "settings.updateAvailable": "有新版本可用",
  "settings.updateVerified": "更新包已验证",
  "settings.diagnostics": "诊断",
  "settings.prepareDiagnostics": "预览诊断报告",
  "settings.exportDiagnostics": "保存诊断报告",
  "settings.diagnosticsHint":
    "导出前先预览内容；报告不包含私人图片，也不会后台上传。",
  "settings.diskHint":
    "达到上限时停止接收新任务；不会自动删除你的原图。预览缓存可按限额清理。",
  "settings.installHint":
    "安装会重启 Orbit。活动任务、配置 journal、凭据 helper 和数据备份检查通过后才可继续。",
  "common.unknown": "未知",
  "common.none": "暂无",
  "common.pending": "待确认",
  "common.unlimited": "不限",
  "common.conflict": "记录冲突",
  "common.loading": "正在读取",
  "common.version": "版本",
  "common.account": "账户",
  "common.confirm": "确认",
  "common.seconds": "秒",
  "common.unavailable": "暂不可用",
  "common.desktopRequired": "请在 Orbit 桌面应用中使用此功能。",
  "common.failed": "操作未完成，请查看连接状态或诊断报告。",
  "common.saved": "已保存",
  "common.copied": "图片已复制",
  "common.deleted": "本地历史已删除",
};
const en: Record<keyof typeof zh, string> = {
  "nav.home": "Home",
  "nav.images": "Images",
  "nav.access": "Access",
  "nav.settings": "Settings",
  "nav.label": "Main navigation",
  "app.subtitle": "Connect your AI workflow",
  "app.synthetic": "Synthetic test data · not actual access or settlement",
  "action.login": "Connect account",
  "action.cancel": "Cancel",
  "action.close": "Close",
  "action.continue": "Continue",
  "action.preview": "Preview configuration",
  "action.apply": "Confirm and apply",
  "action.restore": "Preview restore",
  "action.logout": "Sign out",
  "action.save": "Save",
  "action.copy": "Copy image",
  "action.edit": "Edit again",
  "action.delete": "Delete local history",
  "action.retrySave": "Retry local saving",
  "action.refresh": "Refresh",
  "action.manage": "Manage access",
  "action.more": "More actions",
  "login.title": "Connect to Orivelle",
  "login.description":
    "Sign in and authorize Orbit in your system browser. The authorization flow obtains inference credentials securely.",
  "login.panel": "Panel address",
  "login.service": "Official Orivelle service",
  "login.officialOnly":
    "This version connects only to the official Orivelle service. Please sign in again.",
  "login.hint":
    "Continue to Orivelle sign-in and authorization in your system browser.",
  "login.waiting": "Complete sign-in and authorization in the system browser.",
  "home.eyebrow": "DESKTOP ACCESS",
  "home.title": "Connect your AI tools to Orivelle",
  "home.description":
    "Manage desktop nodes, models and connection settings from one account.",
  "home.account": "Your account",
  "home.loggedOut": "Connect an account to begin",
  "home.loggedOutHint":
    "Use Orivelle sign-in, verification and account permissions. No manual API key is needed.",
  "home.wallet": "Wallet balance",
  "home.budget": "Available budget",
  "home.updatedAt": "Account data updated",
  "login.refreshing": "Refreshing session",
  "login.connected": "Account connected.",
  "login.denied":
    "Browser authorization was denied. The current account is unchanged.",
  "login.expired": "Browser authorization expired. Connect again.",
  "login.cancelled": "Browser authorization cancelled.",
  "common.updateMaintenance":
    "Preparing to install an update. New image requests and configuration changes are paused.",
  "home.balanceUnknown": "Allowance is unconfirmed",
  "home.accountReady": "Account connected",
  "home.permissionDenied":
    "This account cannot obtain automatic connection credentials",
  "home.codex": "Codex desktop",
  "home.claude": "Claude Desktop",
  "home.codexHint":
    "Responses connection · user configuration may be shared with CLI / IDE",
  "home.claudeHint":
    "Third-party inference affects the whole app: Chat, Code and Cowork",
  "home.images": "Start with an image, too",
  "home.imagesHint":
    "Generate, edit and keep local results using an authorized connection.",
  "client.pending": "Detection pending",
  "client.notInstalled": "Application not detected",
  "client.notConfigured": "Not configured",
  "client.saved": "Configuration saved",
  "client.restart": "Restart / load confirmation pending",
  "client.loaded": "Client load confirmed",
  "client.verified": "Connection verified",
  "client.notTested": "Connection not verified",
  "client.failed": "Connection test failed",
  "client.conflict": "Configuration conflict",
  "client.unsupported": "Version or format needs verification",
  "client.managed": "Managed configuration",
  "client.configurationAllowed": "Local user configuration can be previewed",
  "client.configurationBlocked": "Local user configuration writes blocked",
  "client.configurationUnverified": "Local configuration write check pending",
  "client.remotePolicyUnknown":
    "Remote organization policy is unconfirmed and may affect configuration activation.",
  "access.eyebrow": "ACCESS MANAGEMENT",
  "access.title": "Choose a tool. Review every switch.",
  "access.description":
    "Review the account, node, model and impact before applying each desktop configuration.",
  "access.connection": "Current connection",
  "access.node": "Inference node",
  "access.model": "Connection model",
  "access.selectModel": "Select an authorized model",
  "access.noModels": "Connect an account and node to obtain models",
  "access.noConnection": "No account connection",
  "access.noNode": "No available nodes",
  "access.selectNode": "Select an authorized node",
  "access.nodeUnavailable": "Node is currently unavailable",
  "access.readonly":
    "Read-only accounts can view budgets and usage. Automatic credential delivery requires existing business write permissions.",
  "access.previewTitle": "Review configuration and impact",
  "access.previewHint":
    "Apply only this preview, then restart the client completely. Saving does not send an inference request.",
  "access.paths": "Target files",
  "access.impact": "Impact",
  "access.changes": "Redacted changes",
  "access.backups": "Configuration backups",
  "access.noBackups":
    "Applying a configuration creates an encrypted, complete operation backup.",
  "access.backupHint":
    "Keep up to five groups per client for 30 days. External changes stop automatic restore.",
  "access.backupAccount": "Backup account",
  "access.selectBackupAccount":
    "Select the original account and Panel explicitly",
  "access.saved":
    "Configuration saved. Quit and reopen the client; loading and connection remain unconfirmed.",
  "access.restoreLocalConfiguration": "Confirm configuration restore",
  "access.preserveClaudePreferences": "Preview restore keeping preferences",
  "access.preferencesRestoreHint":
    "Only coordinates Claude main-config preference changes. Other fields or configuration-library changes still block restoration.",
  "access.restorePreservingPreferences":
    "Keep preferences and restore connection settings",
  "access.restoredPreservingPreferences":
    "Connection settings restored and later preferences preserved. Quit and reopen the client; loading and connection remain unconfirmed.",
  "access.restored":
    "Original configuration restored. Quit and reopen the client; loading and connection remain unconfirmed.",
  "access.exportImport": "Confirm and export Claude diagnostic candidate",
  "access.importHint":
    "Saves a Claude configuration candidate for inspection. It does not write the client configuration or establish import compatibility. Client configuration, loading and connection status remain unchanged.",
  "access.importExported":
    "Claude diagnostic candidate exported. Client configuration, loading and connection status remain unchanged.",
  "access.previewExpired":
    "This preview is no longer valid, or the account, node, credential, model, organization policy or client version changed. Create a new preview.",
  "access.restorePreviewExpired":
    "This restore preview is no longer valid, or the selected backup was removed, expired or changed. Create a new restore preview.",
  "access.policyUnverified":
    "Local configuration write eligibility or organization restrictions are unconfirmed. Configuration cannot be changed or exported. Check again and create a new preview.",
  "access.policyChanged":
    "Local configuration write eligibility or organization policy changed. This confirmation is no longer valid. Check again and create a new preview.",
  "access.configurationConflict":
    "The target configuration or operation state changed. Overwriting stopped; review the changes before creating a new preview.",
  "access.claudeMappingUnavailable":
    "Claude's local model mapping is unavailable. Keep Orbit running and check whether its local port is in use, then create a new preview.",
  "access.saveLocalConfiguration": "Confirm and save local configuration",
  "access.claudeConfigurationTarget":
    "Installed Claude Desktop configuration target",
  "access.claudeDesktopScope":
    "Third-party inference affects the whole Claude desktop app, including Chat, Code and Cowork. Only the listed target files will change. Restart completely after saving; saving does not confirm loading or connection.",
  "access.claudeConfigurationUnavailable":
    "The configuration contract for this Claude Desktop version is unconfirmed. Saving or restoring is unavailable. Exporting a diagnostic candidate does not confirm configuration activation.",
  "access.claudeVerificationTimeout":
    "Claude app identity verification timed out. The operation stopped; check its status and preview again.",
  "access.claudeNativeMigrationRequired":
    "Claude must migrate the existing local configuration. Check this operation's status, complete migration in Claude, quit it, then preview again.",
  "access.claudeConfigurationChanged":
    "The Claude Desktop configuration target or local write eligibility changed. This confirmation is no longer valid. Create a new preview.",
  "access.claudeRestoreContextUnavailable":
    "This Claude backup has no verifiable recovery context and cannot be restored automatically. Check the original configuration.",
  "access.claudeContextChanged":
    "The Claude configuration account, node, authorization or file state changed. This confirmation is no longer valid. Create a new preview.",
  "access.claudeSourceUnsupported":
    "The current Claude configuration source cannot be migrated safely. Automatic configuration has stopped. Check the current configuration source and follow any recovery prompts.",
  "access.codexConfigurationTarget": "Installed Codex configuration target",
  "access.codexConfigurationCli": "Installed configuration target CLI version",
  "access.codexDefaultScope":
    "Only the default user configuration files below will change. CLI / IDE clients using the same directory may also be affected. Remote, cloud and custom configuration directories are outside this scope.",
  "access.remotePolicyWarning":
    "Remote organization policy is unconfirmed and may affect whether this configuration takes effect. Confirmation changes only the default user configuration; loading and connection remain unverified.",
  "access.codexConfigurationUnavailable":
    "The installed Codex configuration contract or default user configuration path is unconfirmed. Saving or restoring is unavailable. Create a new preview.",
  "access.codexConfigurationChanged":
    "The installed Codex configuration target or local write eligibility changed. This confirmation is no longer valid. Create a new preview.",
  "access.codexCatalog": "Codex model catalog",
  "access.codexMenuAliases": "Model menu aliases in this catalog",
  "access.codexDefaultAlias": "Default model alias",
  "access.codexDefaultEffort": "Default reasoning effort",
  "access.codexDeclaredCli":
    "Declared CLI target (must match the installed configuration target)",
  "access.codexRuntime": "Local backend source check",
  "access.codexRuntimeCli": "Local backend CLI version",
  "access.codexRuntimeVerified":
    "Local backend execution source checked for this preview",
  "access.codexRuntimeUnverified": "Check pending",
  "access.codexRuntimeBlocked":
    "The local runtime source remains unverified. Saving local configuration does not confirm that this runtime instance uses it.",
  "access.codexRuntimeUnavailable":
    "The target Desktop application's local backend execution source has not been verified. Create a new preview and check whether this operation is available.",
  "access.codexRuntimeChanged":
    "The target Desktop application's local backend execution source changed. This confirmation is no longer valid. Create a new preview.",
  "access.codexRestoreContextUnavailable":
    "This backup has no verifiable recovery context and cannot be restored automatically. Check the original configuration manually.",
  "access.codexOverrideUnavailable":
    "Existing explicit settings may override this configuration, so it cannot be applied automatically. Check the settings manually before creating a new preview.",
  "access.codexEvidenceBoundary":
    "An installed version's configuration contract, template declaration and generated catalog do not confirm client loading or connectivity. Restart after saving and confirm them separately.",
  "access.codexBindingUnavailable":
    "This model has no available Codex template declaration. Refresh the model list or ask an administrator to declare one, then preview again.",
  "access.codexReasoningUnavailable":
    "The current reasoning policy cannot produce a complete catalog. Refresh model permissions and preview again.",
  "access.codexCatalogChanged":
    "The model catalog, permissions or connection changed. Refresh and create a new preview.",
  "access.codexCatalogUnavailable":
    "The model catalog cannot be generated safely. Check the template declaration, version and reasoning policy, then preview again.",
  "access.removeHint":
    "Removes local sessions and inference credentials for this connection. History, installation proof and backup keys remain. The server inference key is revoked only if you select that separately.",
  "access.revoke": "Revoke installation inference key",
  "access.remove": "Delete local connection",
  "access.revokeHint":
    "Affects this installation’s desktop and image calls. Other keys stay unchanged; accepted requests may still be billed.",
  "images.eyebrow": "LOCAL IMAGE WORKSPACE",
  "images.title": "Turn an idea into an image",
  "images.description":
    "Choose an authorized model, describe your idea, or add references to edit.",
  "images.create": "Create",
  "images.history": "Local history",
  "images.prompt": "Prompt",
  "images.promptPlaceholder":
    "Describe the subject, composition, lighting and style. Adding references uses the editing endpoint.",
  "images.inputs": "Reference images",
  "images.addInputs": "Add references",
  "images.drop": "Drop PNG, JPEG or WebP images, or select files",
  "images.noInputs": "No reference images",
  "images.params": "Generation parameters",
  "images.count": "Results",
  "images.quality": "Quality",
  "images.size": "Requested dimensions",
  "images.background": "Background",
  "images.default": "Service default",
  "images.generate": "Generate image",
  "images.edit": "Edit image",
  "images.ready": "Ready for your next image",
  "images.empty": "Preview results here",
  "images.emptyHint":
    "Without server stage events, only waiting and elapsed time are shown.",
  "images.noHistory": "No local images yet",
  "images.noHistoryHint":
    "Received images stay local. Deleting history does not change server billing.",
  "images.waiting": "Waiting for response",
  "images.saving": "Saving local results",
  "images.saveFailed": "Local saving failed",
  "images.unconfirmed": "Result unconfirmed",
  "images.failed": "Request failed",
  "images.cancelled": "Locally cancelled",
  "images.completed": "Local results saved",
  "images.cancelHint":
    "Stopping local waiting does not guarantee execution or billing stops.",
  "images.promptLimit": "Prompt exceeds 32768 UTF-8 bytes.",
  "images.requestedModel": "Requested model",
  "images.actualUnknown": "Actual model not observed",
  "images.unverified": "This model’s parameter combination is unverified",
  "images.settlement": "Settlement",
  "images.pendingCharge": "Charges unconfirmed",
  "images.reconciliation": "Reconciliation pending",
  "images.charged": "Settled",
  "images.recorded": "Charge items recorded",
  "images.unbilled": "Server confirmed unbilled",
  "images.incomplete": "Incomplete records",
  "images.priceSales": "Platform sale price",
  "images.priceOfficial": "Official reference",
  "images.priceEstimate": "Operator estimate",
  "images.priceConfiguredReference": "Platform-configured reference",
  "images.pricePartial":
    "Only some meters have configured reference prices; missing rates remain unknown.",
  "images.legacyBilling": "Legacy per-image billing",
  "images.tokenBilling": "Image-token contract billing",
  "images.removeInput": "Remove reference",
  "images.zoom": "Enlarge preview",
  "images.deleteHint":
    "Deletes selected local images and history only; server records and billing are unchanged.",
  "images.inputLimit":
    "Reference count exceeds the selected model limit. Remove some references first.",
  "images.reuse": "Reuse parameters",
  "images.execution": "Server execution record",
  "images.executionConfirmed": "Execution facts confirmed",
  "images.chargeItem": "Charge item",
  "images.pricing": "Prices and billing units",
  "images.unpricedFree":
    "The platform confirms unconfigured pricing under its existing free-admission rule",
  "images.priceHint":
    "Prices use server units. Budgets may use soft admission; Panel records determine settlement. Cached inputs are subsets and are not counted twice.",
  "meter.input": "Input tokens",
  "meter.cachedInput": "Cached input tokens",
  "meter.cacheWrite": "Cache-write tokens",
  "meter.output": "Output tokens",
  "meter.reasoning": "Reasoning tokens",
  "meter.request": "Requests",
  "meter.image": "Image count",
  "meter.textInput": "Text input tokens",
  "meter.cachedText": "Cached text tokens",
  "meter.imageInput": "Image input tokens",
  "meter.cachedImage": "Cached image tokens",
  "meter.imageOutput": "Image output tokens",
  "settings.eyebrow": "PREFERENCES & SECURITY",
  "settings.title": "Make Orbit fit your workflow",
  "settings.description":
    "Manage appearance, files, credential protection and updates here.",
  "settings.appearance": "Appearance and language",
  "settings.theme": "Theme",
  "settings.light": "Light",
  "settings.dark": "Dark",
  "settings.system": "System",
  "settings.language": "Language",
  "settings.files": "Files and disk",
  "settings.disk": "Image disk limit",
  "settings.cache": "Preview cache limit",
  "settings.backups": "Encrypted configuration backups",
  "settings.security": "Secure storage",
  "settings.securityHint":
    "Account sessions, inference credentials, installation identity and backup keys use separate native secure-storage entries. Denied access fails explicitly.",
  "settings.installation": "Installation Panel",
  "settings.logoutHint":
    "Signing out clears account and inference credentials, retaining installation identity and backup keys. Target applications may cache credentials.",
  "settings.forget": "Forget installation",
  "settings.forgetTitle": "Remove installation identity and recovery keys",
  "settings.forgetHint":
    "The original installation proof and related encrypted backups become unrecoverable. Server keys are not automatically revoked.",
  "settings.ackLoss": "I understand related backups cannot be restored",
  "settings.window": "Window behavior",
  "settings.close": "When closing the window",
  "settings.platformDefault": "Follow platform convention",
  "settings.keepRunning": "Keep process and current tasks running",
  "settings.quit": "Quit application",
  "settings.quitHint":
    "Generation is not guaranteed to continue after quitting or crashing. Unconfirmed tasks retain that status.",
  "settings.updates": "Updates",
  "settings.autoCheck": "Check updates on startup",
  "settings.check": "Check updates",
  "settings.download": "Download and verify",
  "settings.install": "Install and restart",
  "settings.disabledUpdate": "Release channel is not configured",
  "settings.upToDate": "Up to date",
  "settings.updateAvailable": "Update available",
  "settings.updateVerified": "Update package verified",
  "settings.diagnostics": "Diagnostics",
  "settings.prepareDiagnostics": "Preview diagnostic report",
  "settings.exportDiagnostics": "Save diagnostic report",
  "settings.diagnosticsHint":
    "Preview before exporting. Reports contain no private images and are not uploaded in the background.",
  "settings.diskHint":
    "New tasks stop at the limit. Original images are not deleted automatically. Preview caches can be trimmed.",
  "settings.installHint":
    "Installing restarts Orbit. Active tasks, configuration journals, credential helpers and data backups must pass the update guard.",
  "common.unknown": "Unknown",
  "common.none": "None",
  "common.pending": "Unconfirmed",
  "common.unlimited": "Unlimited",
  "common.conflict": "Record conflict",
  "common.loading": "Loading",
  "common.version": "Version",
  "common.account": "Account",
  "common.confirm": "Confirm",
  "common.seconds": "s",
  "common.unavailable": "Unavailable",
  "common.desktopRequired": "Open Orbit desktop to use this feature.",
  "common.failed":
    "The operation did not complete. Check the connection or diagnostic report.",
  "common.saved": "Saved",
  "common.copied": "Image copied",
  "common.deleted": "Local history deleted",
};
export type MessageKey = keyof typeof zh;
export function t(key: MessageKey): string {
  return locale.value === "en" ? en[key] : zh[key];
}
