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
| `comment.before_create` | `comment_before_create(ctx)` | `ctx.post_slug`, `ctx.author_name`, `ctx.email`, `ctx.content`, `ctx.parent_id`, `ctx.reply_to_id` | 返回 map：`{action:"allow"}` 放行 / `{action:"block", reason:"..."}` 拦截（评论创建失败，返回 403 `comment_blocked`，reason 进 message）/ 可修改字段后 `{action:"allow", author_name:.., content:..}` |
| `post.after_publish` | `post_after_publish(ctx)` | `ctx.title`, `ctx.slug`, `ctx.published_at` | 返回值忽略（通知类钩子，如触发外部 webhook 由插件自行实现受限能力——第一版无网络能力，仅用于日志/内部状态） |

- 多个启用的插件实现同一钩子：按插件 slug 字典序**依次串行**调用，前一个的输出 map 作为下一个的输入（链式）。
- 渲染管线与公式（2026-10-04 补）：文章正文（`GET /api/posts/:slug` 的 `content_md`，含 `post.before_render`
  改写后的结果）由**前端**渲染，其中 `$…$` 行内 / `$$…$$` 块级数学公式由前端 KaTeX 渲染
  （api-contract.md「文章公式渲染」）。插件改写 content_md 不影响公式渲染能力，也无需感知公式语法；
  `post.after_render` 的 content_html 供页面（page）详情注入，与公式渲染链路互不影响。
- `comment.before_create` 链式时，任一插件返回 `block` 立即短路拦截。
- `comment.before_create` 对**回复（楼中楼）同样生效**（2026-10-03 嵌套评论新增，见 api-contract.md「评论回复」）：
  - `ctx.parent_id` / `ctx.reply_to_id` 为 INT，**0 表示无**；值是父评论校验与两级归一化
    **之后**的最终存储值（parent_id=顶级楼层 id；reply_to_id=被回复的中间楼层 id，
    直接回复顶级评论或发顶级评论时为 0）
  - 这两个字段只读：钩子返回 map 中修改 parent_id/reply_to_id 不影响存储；
    可改写字段仍为 author_name/email/content
  - 顶级评论（非回复）创建时两者均为 0，老插件脚本不读取它们也不受影响
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
- **主题设置**（2026-10-03 新增，见下节）：激活主题的生效设置在令牌之后应用——
  值写入 `--theme-setting-*` CSS 变量与 `data-setting-*` / `data-layout` 属性，
  是令牌管线之上的**叠加层**，不改动 tokens/tokens_dark 机制。

### 主题设置项（theme.toml `[[settings]]`，2026-10-03 新增）

每个主题可在 theme.toml 中声明自己的可配置项（仿 Typecho「外观 → 设置」），
后台按声明渲染设置面板，值按主题 slug 独立持久化。

```toml
[[settings]]
key = "layout"                    # 必填：^[a-z0-9][a-z0-9_-]{0,63}$，同一主题内唯一
label = "页面布局"                 # 可选：面板显示名；缺省用 key
type = "select"                   # 必填：text | textarea | color | select | switch | number
group = "布局"                    # 可选：面板分组标题；缺省不分组
default = "topbar-two-column"     # 可选：默认值（无声明默认值时该设置为"空"，前端回退）
options = [                       # select 必填且非空；其余类型必须缺省
  { value = "topbar-two-column", label = "顶栏导航 + 双列" },
  { value = "topbar-minimal-three-column", label = "极简顶栏 + 三列" },
]
```

- `options` 元素支持两种写法：字符串（value=label）或 `{value, label}` 内联表；
  后端下发时**统一归一化为 `[{value, label}]`**。
- `default` 的类型必须与 `type` 匹配：switch → bool；number → 整数或小数；
  select → options 中的某个 value；color → `#RGB`/`#RRGGBB`/`#RRGGBBAA`（大小写不敏感）；
  text/textarea → 字符串。
- 声明校验（上传 zip 与后端解析共用）：key 非法/重复、type 未知、select 缺 options、
  default 与类型不符、text 类 default 超长度限制 → 上传时 422 `invalid_manifest`。
- 未声明 `[[settings]]` 的主题：接口返回空 `settings` 数组，面板显示
  「该主题无自定义设置项」。

**值的存储与类型转换**（PUT 保存时）：
- 一律以规范化 TEXT 存入 `theme_settings` 表：switch → `"true"`/`"false"`
  （也接受 JSON bool 与 `"true"`/`"false"` 字符串输入）；number → 规范化数字串
  （接受 JSON number 或数字字符串）；color → 小写 hex；text/textarea/select → 原样字符串。
- 长度上限：text ≤500 字符、textarea ≤5000 字符（按 char 计）。
- 校验失败：key 未在声明内 → 422 `unknown_setting`；值与类型不符 / select 越界 /
  color 非 hex / 超长 → 422 `invalid_value`。

**生效值下发**（GET 响应中的 `values`）：声明的 `default` 与已存值合并
（已存值优先），并按类型输出 JSON：switch → bool、number → number、其余 → string。

**内置布局设置**（frontend 消费的系统级约定）：default 主题声明
`key = "layout"` 的 select 设置，前端认识两个值：
- `topbar-two-column`（默认）：顶栏导航 + 主内容/侧栏双列；
- `topbar-minimal-three-column`：顶栏仅保留搜索与后台管理入口，正文左中右三列
  （左：导航/分类，中：文章流，右：侧栏信息）。
其他主题也可声明同 key 的 layout 设置复用这两套骨架；未声明 layout 或值未知时，
前端回退 `topbar-two-column`。

**前端应用约定**：
- 每个设置值写入 `:root` 内联 CSS 变量 `--theme-setting-<key>`（key 中 `_` 换成 `-`，
  如 `accent_color` → `--theme-setting-accent-color`）；theme.css 与内置组件用
  `var(--theme-setting-…)` 消费，未设置时变量不存在、`var()` 自然回退。
- 同时写 `data-setting-<key-dashes>="字符串化值"` 属性；layout 生效值额外写
  `data-layout="<layout 值>"`（供布局组件与 theme.css 选择器读取）。
- 切换/保存设置后立即重新拉取并应用，无需刷新页面；换主题时先清掉上一主题的
  全部 `--theme-setting-*` 变量与 `data-setting-*` 属性再写入新值。

### 主题设置 API

公开（未安装门禁白名单，见第三部分）：
- `GET /api/themes/:slug/settings` → 200
  `{slug, settings: [ThemeSetting], values: {key: value}}`
  - `ThemeSetting = {key, label, type, group?, default?, options?}`（options 已归一化）
  - slug 磁盘上不存在（theme.toml 缺失/非法）→ 404 `not_found`；
    `default` 主题在磁盘缺失时用后端内置常量兜底（同 themes/active 的兜底顺序）
  - 未安装状态：DB 不可用，`values` = 声明默认值（保证安装页能拿到 default 主题设置）

管理（需 Bearer）：
- `GET /api/admin/themes/active/settings-panel` → 200
  `{slug, name, settings, values}`（当前**激活主题**的声明 + 已存值合并；
  管理入口只服务激活主题）
- `PUT /api/admin/themes/:slug/settings` body `{values: {key: value, ...}}` → 200
  响应与公开 GET 同形状（合并后的最新生效值）
  - slug 未安装 → 404 `not_found`；`values` 为空对象 → 200（无改动）
  - 校验见上节（422 `unknown_setting` / `invalid_value`）
  - 部分更新语义：仅写入请求中出现的 key，未出现的已存值保持不变

**生命周期**：主题设置按 slug 隔离——切换激活主题不清除任何主题的设置；
**删除（卸载）主题时连带删除其 theme_settings 行**（重装后回到声明默认值）。

### 主题组件（widgets，2026-10-03 新增）

组件（widget）= 前台布局区域（侧栏/左栏/右栏/页脚）中可启用、可排序、可配参数的
展示单元。接口与存储契约见 api-contract.md「主题组件」；本节定义主题包侧的声明规格。

**三类组件**：
1. **内置组件**（kind=builtin）：reedblog 前端逐一实现的 React 组件，key 固定为
   `recent-posts` / `hot-posts` / `tag-cloud` / `categories` / `archive` / `links` /
   `site-info`；任何主题都可配置它们（启用/位置/排序/参数），后端注册表提供
   参数 schema（params）与默认值；
2. **主题声明组件**（kind=custom、source=theme）：主题在 theme.toml 用 `[[widgets]]`
   声明、HTML 片段放在主题包 `assets/widgets/<key>.html`——第三方主题无需前端实现
   即可带自定义组件（HTML 片段型兜底）；
3. **后台自建组件**（kind=custom、source=admin）：管理员在后台「主题设置 → 组件管理」
   直接创建（名称 + HTML 内容 + 位置 + 排序），HTML 存 config.html，可增删改。

三类组件**统一走同一份渲染管线与安全策略**：custom 组件的 HTML 由前端以
template + DOM 注入方式渲染（`<script>` 重建为可执行元素），与插件前端轻注入同款
（不转义，站长/主题作者对自己内容负责，WordPress 同信任模型）；
`dangerouslySetInnerHTML` 级别的原始注入仅用于此类后台/主题包输入的片段。

**theme.toml `[[widgets]]` 声明**（可选；未声明的主题只有内置组件可配）：

```toml
[[widgets]]
key = "notice"                # 必填：^[a-z0-9][a-z0-9_-]{0,63}$，主题内唯一，
                              # 不得与内置组件 key 冲突（违规 → 上传 422 invalid_manifest）
label = "公告栏"               # 可选：显示名，缺省用 key
default_enabled = false       # 可选：默认是否启用（缺省 false）
default_position = "footer"   # 可选：默认位置（sidebar|left|right|footer，缺省 sidebar）
default_sort = 50             # 可选：默认排序（缺省 100）

[[widgets.params]]            # 可选：参数 schema，复用 [[settings]] 的声明形状与校验
key = "text"                  # （text|textarea|color|select|switch|number + default/options）
label = "公告文字"
type = "text"
default = "欢迎来到本站"
```

- HTML 片段路径固定：`assets/widgets/<key>.html`（经既有 assets 端点托管；文件缺失时
  html 为空串，后台可用 config.html 覆盖补上）；
- **`{{param}}` 令牌替换**：片段中的 `{{<参数 key>}}` 在公开接口输出 config.html 时
  替换为该参数的生效值（已存值优先、声明 default 兜底；不转义）；
- 声明校验（上传 zip 与后端解析共用）：key 非法/重复/与内置冲突、default_position
  越界、params 违规（同 [[settings]] 规则）→ 422 `invalid_manifest`；
- 主题声明组件的参数编辑、启用/位置/排序与内置组件在后台同一面板操作；
  **删除主题时其 widgets 配置行连带删除**（同 theme_settings 生命周期）。

### 主题管理 API
公开：
- `GET /api/themes/active` → `{slug, name, tokens:{...}, tokens_dark?:{...}, css_url:"/api/themes/:slug/theme.css"|null, preview_url?:str}`（未安装门禁白名单：未安装时返回 default 主题令牌，保证安装页有样式）
- `GET /api/themes/:slug/theme.css` → `text/css`（不存在 404）
- `GET /api/themes/:slug/assets/*path` → 静态文件（不存在 404；防目录穿越）
- `GET /api/themes/:slug/widgets` → 生效组件配置（白名单；完整条款见 api-contract.md「主题组件」）

组件管理（需 Bearer；完整条款见 api-contract.md「主题组件」）：
- `GET /api/admin/themes/:slug/widgets` → 全量组件配置（默认值合并已存行）
- `PUT /api/admin/themes/:slug/widgets` → 全量替换保存（校验失败 422，不写库）

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
- `GET /api/themes/:slug/settings`（2026-10-03 主题设置新增：未安装时 values=声明默认值，
  保证安装页能应用 default 主题的布局/配色设置；磁盘不存在的 slug 仍 404）
- `GET /api/themes/:slug/widgets`（2026-10-03 组件系统新增：未安装时返回内置组件默认
  启用集，保证安装页/首装前台可渲染；磁盘不存在的 slug 仍 404）
- `GET /api/frontend/injections`

（原有白名单：`GET /api/health`、`GET /api/install/status`、`POST /api/install` 不变。）

## 第四部分：config.toml 扩展
```toml
[plugins]
dir = "plugins"

[themes]
dir = "themes"
active = "default"

# 邮件通知（2026-10-04 新增）：只在这里存 SMTP 密码（安全默认，绝不入库、
# 绝不经 API 返回）；其余 SMTP 项在后台「邮件通知」页配置（settings 表）。
# 环境变量 REEDBLOG_SMTP_PASSWORD 优先于本项。
[smtp]
password = ""
```

## 第五部分：数据表新增
- `plugins`：slug TEXT PK, enabled INTEGER/BOOL, installed_at, updated_at
- 主题本体不入 DB（磁盘 + config.toml active 足够）；active 权威来源是 config.toml。
- `theme_settings`（2026-10-03 主题设置新增）：theme_slug, key, value, updated_at；
  主键 (theme_slug, key)，按主题 slug 隔离；value 为规范化 TEXT，
  时间戳沿用全库 RFC3339 UTC 文本惯例；SQLite/MySQL 共用 SQL（Any 驱动，
  `key` 列名用反引号引用——双方言均支持）。删除主题时连带删除其行。
- `theme_widgets`（2026-10-03 组件系统新增）：id 自增主键，theme_slug, widget_key
  （UNIQUE (theme_slug, widget_key)）, enabled, position, sort_order, config（JSON 对象
  TEXT）, created_at, updated_at。**只存覆盖行**：内置/主题声明组件的默认值来自
  后端注册表与 theme.toml 声明，未保存过的组件不占行；按主题 slug 隔离，
  删除主题时连带删除其行。时间戳沿用全库 RFC3339 UTC 文本惯例；
  SQLite/MySQL 共用 SQL（Any 驱动）。

## 迁移注意
- 新增 `plugins` / `theme_settings` 表走 sqlx migration（SQLite + MySQL 各一份），migration 需对**已有安装**幂等（新表，不影响旧数据）。
- 已安装站点升级后：首次启动自动补建 default 主题目录、建 plugins / theme_settings 表。
