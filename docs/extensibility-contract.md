# reedblog 扩展系统契约 v1（插件 + 主题）

本文件定义插件系统与主题系统的前后端共同规格，与 `docs/api-contract.md`（核心 API）配套。后端（backend/）与前端（frontend/）都必须以此为准；如需偏离，在交付报告显式列出。

约定沿用核心契约：全部 JSON/UTF-8、RFC3339 UTC 时间戳、错误形状 `{"error":{"code","message"}}`、分页 `{"items","total","page","per_page"}`、鉴权 JWT Bearer。管理端点全部需 Bearer。

---

## 第一部分：插件系统

### 设计目标
- 插件 = 一个目录，作者只写 **Rhai 脚本 + manifest**，无需编译、无需 Rust 知识。
- 安全沙箱：Rhai 引擎关闭危险能力（无文件 IO、无网络、无 `print` 到 stdout 之外的副作用、限制调用深度/循环次数/字符串长度），插件只能操作传入的数据并通过返回值影响流程。
- 热重载：后台启用/停用/重装插件不需要重启后端进程。
- 作用范围：**后端钩子**（可读写数据、可拦截）+ **前端轻注入**（往页面塞 HTML 片段/脚本/widget），前端**不注入 React 组件**。

### 插件目录结构
```
plugins/
  <plugin-slug>/
    manifest.toml        # 必需，元数据 + 声明钩子
    main.rhai            # 必需，入口脚本（钩子函数定义在此）
    inject/              # 可选，前端注入片段
      head.html          # 注入 <head>（统计脚本、meta、CSS link 等）
      body_end.html      # 注入 </body> 前（widget、悬浮组件等）
    README.md            # 可选
```
- 插件存储根目录：后端运行目录下 `plugins/`（config.toml `[plugins] dir` 可配，默认 `plugins`）。
- 上传安装：zip 包内根层必须是单个插件目录（含 manifest.toml）。

### manifest.toml
```toml
name = "示例插件"          # 显示名
slug = "demo-plugin"       # 唯一标识，须与目录名一致，^[a-z0-9-]+$
version = "1.0.0"          # semver
description = "一句话说明"
author = "作者名"
min_app_version = "0.1.0"  # 可选，低于此 reedblog 版本拒绝启用
# 声明本插件实现的钩子（后端据此路由；未声明的钩子即使脚本里定义了也不调用）
hooks = ["post.before_render", "post.after_render", "comment.before_create", "post.after_publish"]
# 前端注入位置声明（可选）；未声明则不注入
inject = ["head", "body_end"]
```
- slug 唯一性：与已安装插件冲突 → 409 `plugin_exists`。
- hooks / inject 只能是下方枚举里的值；出现未知值 → 422 `invalid_manifest`。

### 后端钩子清单（Rhai 函数签名）
每个钩子是 main.rhai 里的一个函数，名字与钩子 id 对应（`.`→`_`，如 `post.before_render` → 函数 `post_before_render`）。入参是一个 Rhai object（map），返回值决定行为。

| 钩子 id | Rhai 函数 | 入参字段 | 返回值语义 |
|---|---|---|---|
| `post.before_render` | `post_before_render(ctx)` | `ctx.title`, `ctx.content_md`, `ctx.slug` | 返回修改后的 map（同结构），用于改写待渲染内容；返回原样即不改 |
| `post.after_render` | `post_after_render(ctx)` | `ctx.title`, `ctx.content_html`, `ctx.slug` | 返回修改后的 map，用于改写渲染后的 HTML（如追加版权声明） |
| `comment.before_create` | `comment_before_create(ctx)` | `ctx.post_slug`, `ctx.author_name`, `ctx.email`, `ctx.content` | 返回 map：`{action:"allow"}` 放行 / `{action:"block", reason:"..."}` 拦截（评论创建失败，返回 403 `comment_blocked`，reason 进 message）/ 可修改字段后 `{action:"allow", author_name:.., content:..}` |
| `post.after_publish` | `post_after_publish(ctx)` | `ctx.title`, `ctx.slug`, `ctx.published_at` | 返回值忽略（通知类钩子，如触发外部 webhook 由插件自行实现受限能力——第一版无网络能力，仅用于日志/内部状态） |

- 多个启用的插件实现同一钩子：按插件 slug 字典序**依次串行**调用，前一个的输出 map 作为下一个的输入（链式）。
- `comment.before_create` 链式时，任一插件返回 `block` 立即短路拦截。
- Rhai 脚本运行时错误/超时：记录日志，该插件该钩子**跳过**（不阻断主流程），并在插件详情 `last_error` 字段暴露。
- 沙箱限制（后端强制）：`Engine::set_max_call_levels(64)`、`set_max_operations(50000)`、`set_max_string_size(1MB)`、`set_max_array_size`/`map_size` 合理上限；不注册任何文件/网络/进程 API。

### 前端轻注入
- 前端在渲染公开博客页面（不含 /admin、/install）时，请求 `GET /api/frontend/injections`，拿到所有**启用且声明了 inject** 的插件片段，按位置注入：
  - `head` 片段 → 注入文档 `<head>`（用 portal 或直接 innerHTML 追加到 head）
  - `body_end` 片段 → 注入 `<body>` 末尾
- 注入内容是**原始 HTML 字符串**（可含 `<script>`、`<style>`、`<link>`）。前端用 `dangerouslySetInnerHTML` 或 DOM 注入，**不做转义**（插件作者对自己内容负责，与 WordPress 插件同信任模型）。
- 注入在客户端 hydration 后执行一次；片段变更（后台改动）后刷新页面生效。
- /admin 和 /install **不注入**任何插件片段（避免干扰后台/安装流程）。

### 插件管理 API（均需 Bearer）
- `GET /api/admin/plugins` → `{"items":[PluginInfo], "total":int}`（插件数量少，不分页但保留形状）
  - `PluginInfo = {slug, name, version, description, author, enabled:bool, hooks:[str], inject:[str], min_app_version?, last_error?, installed_at, updated_at}`
- `POST /api/admin/plugins` （multipart/form-data，字段 `file` = zip）→ 201 `PluginInfo`
  - 校验：zip 解压后根层单目录且含 manifest.toml；slug 合法且未占用；manifest 合法（否则 422 `invalid_manifest`）；slug 冲突 409 `plugin_exists`；zip 非法 422 `invalid_package`
  - 安装后默认 `enabled:false`（需显式启用，防误触发）
- `GET /api/admin/plugins/:slug` → `PluginInfo`（含 last_error）
- `POST /api/admin/plugins/:slug/enable` → 200 `PluginInfo`（enabled:true；加载并校验 Rhai 脚本，语法错误则 422 `script_error` 且不启用）
- `POST /api/admin/plugins/:slug/disable` → 200 `PluginInfo`（enabled:false）
- `DELETE /api/admin/plugins/:slug` → 204（删除目录；正在启用的也可直接删）
- 插件启用状态持久化：存 SQLite/MySQL 一张 `plugins` 表（slug, enabled, installed_at, updated_at）；manifest 内容以磁盘文件为准，DB 只存开关与时间戳。启动时加载所有磁盘插件，按 DB 记录恢复 enabled 状态；DB 无记录的视为未启用。

### 前端插件片段 API（公开，无需鉴权）
- `GET /api/frontend/injections` → `{"head":[{plugin:"slug", html:"..."}], "body_end":[...]}`
  - 仅返回 enabled 且声明了对应 inject 的插件；未安装任何插件时返回空数组。
  - 此端点在**未安装状态**下也应可用（返回空），避免前端 InstallGate 之外的页面因它 503。→ 归入 not_installed 门禁白名单。

---

## 第二部分：主题系统

### 设计目标
- 主题 = 一个目录/zip，含设计令牌 + 自定义 CSS + 静态资源；第三方可开发、后台上传切换。
- 前端启动时拉取激活主题并应用：设计令牌覆盖 CSS 变量（shadcn 主题变量体系）+ 加载主题的 theme.css + 字体等 assets。
- 至少内置 1 套默认主题（default），删不掉、不可停用，保证任何情况下站点有样式。

### 主题目录结构
```
themes/
  <theme-slug>/
    theme.toml           # 必需，元数据 + 设计令牌
    theme.css            # 可选，自定义 CSS（可覆盖组件类）
    preview.png          # 可选，后台预览图
    assets/              # 可选，字体/图片等，经 /api/themes/:slug/assets/* 暴露
```
- 存储根目录：后端运行目录下 `themes/`（config.toml `[themes] dir` 可配，默认 `themes`）。

### theme.toml
```toml
name = "默认主题"
slug = "default"           # 唯一，须与目录名一致，^[a-z0-9-]+$
version = "1.0.0"
description = "内置简洁浅色主题"
author = "reedblog"
# 设计令牌：覆盖 shadcn/Tailwind 的 CSS 变量（HSL 分量，不带 hsl() 包裹）
[tokens]
background = "0 0% 100%"
foreground = "240 10% 3.9%"
primary = "240 5.9% 10%"
primary_foreground = "0 0% 98%"
secondary = "240 4.8% 95.9%"
muted = "240 4.8% 95.9%"
muted_foreground = "240 3.8% 46.1%"
accent = "240 4.8% 95.9%"
border = "240 5.9% 90%"
ring = "240 5.9% 10%"
radius = "0.5rem"
# 可选：暗色变体令牌（若主题提供）
[tokens_dark]
background = "240 10% 3.9%"
# ...
```
- 令牌 key 为 CSS 变量名去掉 `--` 前缀的下划线形式，后端原样下发；前端映射为 `--<key 中 _ 换成 ->`（如 `primary_foreground` → `--primary-foreground`）。`radius` 特殊，映射 `--radius`，值带单位。
- 未知 key 忽略（向前兼容）；缺失 key 用前端内置默认值兜底。

### 主题应用机制（前端）
- 前端启动（公开站点与后台都）时 `GET /api/themes/active`，拿到激活主题的令牌 + theme.css 的 URL。
- 令牌：写入 `:root` 的 CSS 变量（document.documentElement.style.setProperty）。
- theme.css：以 `<link rel="stylesheet" href="/api/themes/:slug/theme.css">` 注入 head。
- assets：theme.css 里引用 `/api/themes/:slug/assets/xxx` 绝对路径，后端静态托管。
- 切换主题：后台设置激活主题 → 写入 config.toml `[theme] active = "slug"` → 前端下次加载生效（前端可主动重新拉取 active 并热切换，非强制）。

### 主题管理 API
公开：
- `GET /api/themes/active` → `{slug, name, tokens:{...}, tokens_dark?:{...}, css_url:"/api/themes/:slug/theme.css"|null, preview_url?:str}`（未安装门禁白名单：未安装时返回 default 主题令牌，保证安装页有样式）
- `GET /api/themes/:slug/theme.css` → `text/css`（不存在 404）
- `GET /api/themes/:slug/assets/*path` → 静态文件（不存在 404；防目录穿越）

管理（需 Bearer）：
- `GET /api/admin/themes` → `{"items":[ThemeInfo], "total":int}`
  - `ThemeInfo = {slug, name, version, description, author, active:bool, builtin:bool, has_css:bool, preview_url?:str, installed_at, updated_at}`
  - `builtin:true` 仅 default 主题（内置、不可删除）
- `POST /api/admin/themes`（multipart `file`=zip）→ 201 `ThemeInfo`
  - 校验同插件：根层单目录含 theme.toml；slug 合法未占用（409 `theme_exists`）；manifest 非法 422 `invalid_manifest`；zip 非法 422 `invalid_package`
  - default 主题不可被上传覆盖 → 409 `builtin_protected`
- `POST /api/admin/themes/:slug/activate` → 200 `ThemeInfo`（写 config.toml active）
- `DELETE /api/admin/themes/:slug` → 204；default（builtin）不可删 → 409 `builtin_protected`；当前激活主题不可删 → 409 `theme_active`（需先切换）

### 内置 default 主题
- 后端首次运行（安装时）若 `themes/default` 不存在，自动生成一套 default 主题（theme.toml 用 shadcn 默认 neutral 令牌，无 theme.css 或空 css）。
- 保证 `GET /api/themes/active` 在任何情况下（含未安装）都能返回 default，前端永远有基础样式。

---

## 第三部分：门禁白名单更新
not_installed 中间件白名单**新增**（未安装也可访问）：
- `GET /api/themes/active`、`GET /api/themes/:slug/theme.css`、`GET /api/themes/:slug/assets/*`
- `GET /api/frontend/injections`

（原有白名单：`GET /api/health`、`GET /api/install/status`、`POST /api/install` 不变。）

## 第四部分：config.toml 扩展
```toml
[plugins]
dir = "plugins"

[themes]
dir = "themes"
active = "default"
```

## 第五部分：数据表新增
- `plugins`：slug TEXT PK, enabled INTEGER/BOOL, installed_at, updated_at
- 主题不入 DB（磁盘 + config.toml active 足够）；如需缓存可加，但 active 权威来源是 config.toml。

## 迁移注意
- 新增 `plugins` 表走 sqlx migration（SQLite + MySQL 各一份），migration 需对**已有安装**幂等（新表，不影响旧数据）。
- 已安装站点升级后：首次启动自动补建 default 主题目录、建 plugins 表。
