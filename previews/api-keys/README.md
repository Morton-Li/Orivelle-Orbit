# API Key 界面预览

独立 Vue 预览，复用 Orbit 的样式、图标、选择器、弹窗及状态组件。全部数据与交互均在内存中模拟，不调用 Panel、系统安全存储、剪贴板或桌面客户端配置。

在工作区根目录使用已有依赖运行：

```sh
node_modules/.bin/vite --config previews/api-keys/vite.config.ts
```

访问 `http://127.0.0.1:1436/`。可操作列表、搜索、筛选、详情、新建与编辑、默认选择、秘密揭示、轮换、撤销、删除和模拟接入更新；底部可切换只读及空状态，顶栏可切换主题和重置预览。刷新页面恢复初始数据。

预览专用检查：

```sh
node_modules/.bin/vue-tsc --noEmit -p previews/api-keys/tsconfig.json
node_modules/.bin/vite build --config previews/api-keys/vite.config.ts
```

此入口不进入正式产品路由或生产构建。
