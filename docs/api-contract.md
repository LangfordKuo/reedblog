# reedblog API 契约 v1

前后端共同规格。后端（backend/，Rust+Axum）与前端（frontend/，React）都必须以此为准；如需偏离，必须在交付报告里显式列出。

## 总则
- Base URL：`/api`（开发时前端 Vite 代理到 `http://localhost:3000`；生产由 Nginx 反代）
- 全部 JSON，UTF-8。时间戳一律 RFC3339（UTC），如 `2026-10-02T12:00:00Z`
- 错误统一形状：`{"error": {"code": "<snake_case>", "message": "<人类可读>"}}`，配合恰当 HTTP 状态码
- 鉴权：JWT Bearer。请求头 `Authorization: Bearer ***`
- 分页响应统一：`{"items": [...], "total": <int>, "page": <int>, "per_page": <int>}`；分页参数 `page`（默认1）、`per_page`（默认10，上限100）。
  公开文章列表（`/api/posts`、`/api/search`）**未传 per_page 时默认值取站点设置的 per_page**（2026-10-03 定，见「站点设置」）；显式传入的 per_page 优先，仍钳制 1~100

## 数据形状
```
SiteInfo     = {title, subtitle, installed: bool}
PostPublic   = {id, title, slug, excerpt, category: {id, name}|null,
                tags: [{id, name}], published_at, comment_count}
PostDetail   = PostPublic + {content_md}
SearchResult = PostPublic + {snippet}   // snippet 为纯文本上下文片段，见「全文搜索」
PostAdmin    = {id, title, slug, content_md, excerpt, status: "draft"|"published",
                category_id|null, category_name|null, tag_ids: [int],
                published_at|null, created_at, updated_at}
Category     = {id, name, post_count}
Tag          = {id, name, post_count}
CommentPub   = {id, author_name, content, created_at}
CommentAdmin = {id, post_id, post_title, author_name, email|null, content,
                status: "approved"|"hidden", created_at,
                target_type: "post"|"page"}   // 2026-10-03 页面功能扩展，见「页面」
PageKind     = "custom" | "message_board" | "links"
PageSummary  = {id, title, slug, kind: PageKind, sort_order}
PageLink     = {id, name, url, description, sort_order}
PageDetail   = {id, title, slug, kind, content_html, sort_order, updated_at,
                links: [PageLink]}   // links 仅 kind=links 时非空，其余为 []
PageAdmin    = {id, title, slug, kind, content_md, enabled: bool, sort_order,
                built_in: bool, links: [PageLink], created_at, updated_at}
AuthResult   = {token, username, expires_at}
UploadResult = {url, size: <字节数>, filename: <原始文件名回显>}
SiteSettingsPublic = {title, subtitle, description, icp_number, footer_text, per_page: int}
SiteSettingsAdmin  = SiteSettingsPublic + {base_url}   // base_url 为敏感字段，公开接口不返回
```

## 安装向导（未初始化时）
- `GET /api/health` → `{"status": "ok"}`（永远可用）
- `GET /api/install/status` → `{"installed": bool}`（永远可用）
- `POST /api/install` → 201 `{"ok": true}`
  - body: `{db_type: "sqlite"|"mysql", sqlite_path?: string(默认 "reedblog.db"), mysql?: {host, port, username, password, database}, admin: {username, password}, site: {title, subtitle?}}`
  - 行为：验证连接 → 写入 `backend/config.toml` → 建表（按 db_type 跑对应迁移）→ 创建管理员（argon2 哈希）→ 生成 JWT secret 存 config
  - **安装完成时自动注入示例分类/标签/文章/评论**（2026-10-03 定）：仅安装流程执行一次，正常启动路径绝不重复注入；注入失败只记 warning 日志、安装照常成功；响应形状不变（仍为 201 `{"ok": true}`）
  - **安装完成时自动注入 3 个内置页面**（2026-10-03 定，见「页面」）：关于（/about）、留言板（/guestbook，kind=message_board）、友情链接（/links，kind=links）。内置页面属功能性数据而非示例内容：**安装路径与正常启动路径都会按 slug 幂等补齐**（已存在的行绝不覆盖，管理员的编辑/停用不受影响）；注入失败只记 warning 日志
  - 已安装后再调 → 409 `{"error":{"code":"already_installed",...}}`
- **未安装状态下**，除 `/api/health`、`/api/install/status`、`POST /api/install` 外的所有 `/api/*` 返回 503 `{"error":{"code":"not_installed",...}}`
- 安装完成无需重启进程（进程内切换到已初始化状态即可；实现上允许重启，但接口行为必须一致）

## 站点公开接口（已安装后可用）
- `GET /api/site` → `SiteInfo`（title/subtitle 与站点设置一致，见「站点设置」）
- `GET /api/posts?page&per_page&tag=<name>&category=<name>&year=<int>&month=<int>` → 分页 `[PostPublic]`，仅 published，按 published_at DESC；列表不含 content_md
- excerpt 为空时的回退（2026-10-03 定）：由 content_md 生成**纯文本**摘要（剥离 Markdown 语法：标题#、强调符、代码围栏、表格线、链接保留文字），截断至 ≤200 字符；不得返回含 Markdown 符号的原文
- `GET /api/posts/:slug` → `PostDetail`；不存在/未发布 → 404 `not_found`
- `GET /api/posts/:slug/comments` → `[CommentPub]`（仅 approved，按时间 ASC）
- `POST /api/posts/:slug/comments` → 201 `CommentPub`
  - body: `{author_name, email?, content}`；必填校验 422 `validation_error`
  - 默认先发后审：创建即 approved
- `GET /api/tags` → `[Tag]`（post_count 只统计 published）
- `GET /api/categories` → `[Category]`（同上）
- `GET /api/archive` → `[{"year": int, "month": int, "count": int}]`，仅 published，按年月 DESC

全文搜索（2026-10-03 新增）：
- `GET /api/search?q=<关键词>&page&per_page` → 分页 `[SearchResult]`（复用总则分页壳），
  仅 published，按 published_at DESC；分页参数与 `/api/posts` 相同
  - 分词：q 按空白切分为词条（上限 8 个，多余忽略），**每个词条都必须命中**（AND 语义）；
    匹配范围：title、excerpt、content_md 三列
  - 实现约束：LIKE（SQLite/MySQL 共用一份 SQL，零迁移；**禁止** FTS5、MATCH…AGAINST 等单方言语法）；
    词条内 LIKE 通配符 `%`、`_`、`\` 先行转义，配合 `ESCAPE '\'` 子句（双方言通用）
  - 大小写：英文匹配大小写不敏感（依赖双方言 LIKE 的默认 CI 排序规则）
  - q 缺失或 trim 后为空 → 400 `validation_error`
  - snippet：把 content_md 按「excerpt 为空时的回退」同款规则剥成纯文本（不含任何 HTML/Markdown
    markup），定位**任一词条的首个命中位置**（大小写不敏感），截取命中点前 ≤40 字符、后 ≤60 字符
    的窗口（按 char 计，CJK 安全）；窗口两端非文本边界时补 `…`；纯文本中找不到命中
    （仅 title/excerpt 命中）则回退为响应中的 excerpt。命中高亮由前端自行实现

上传文件读取（2026-10-03 新增）：
- `GET /api/uploads/*path` → 已上传图片的静态读取（公开、无需鉴权；不在未安装门禁白名单内，未安装时同样 503）
  - Content-Type 按扩展名（png/jpg/gif/webp）；响应带 `Cache-Control: public, max-age=31536000, immutable`
  - 防目录穿越（词法清洗 + canonicalize 物理校验，同 themes assets）；文件不存在/路径非法 → 404

RSS 与 sitemap（2026-10-03 新增；base_url 来源 2026-10-03 更新为站点设置优先）：
- 站点绝对 URL 来源（feed/sitemap 共用，按优先级）：
  1. 站点设置 `base_url` 非空则用它（去尾 `/`，见「站点设置」）
  2. 否则 config.toml `[server] base_url` 非空则用它（去尾 `/`）
  3. 否则从请求头推导：scheme 取 `X-Forwarded-Proto`（缺省 `http`，仅接受 http/https），
     host 取 `X-Forwarded-Host` → `Host`（反代场景）
- feed channel 的 title/description 读站点设置（title=站点名称；description=副标题，为空回退标题）
- `GET /api/feed.xml` → RSS 2.0，Content-Type `application/rss+xml; charset=utf-8`
  - 最新 20 篇 published 文章，按 published_at DESC
  - channel 含 title、link（站点绝对 URL）、description（来源见上）
  - item 含 title、link（`{base}/posts/{slug}`，与前端路由一致）、guid（isPermaLink=true，同 link）、
    pubDate（RFC 822）、description（excerpt，为空时按「excerpt 回退」条款从正文推导）
  - 所有文本 XML 转义（`& < > " '`）
- `GET /api/sitemap.xml` → urlset（xmlns `http://www.sitemaps.org/schemas/sitemap/0.9`），
  Content-Type `application/xml; charset=utf-8`
  - 含：首页 `{base}/`、全部 published 文章详情页（lastmod=updated_at，W3C datetime 即 RFC3339）、
    全部 **enabled 页面**（2026-10-03 页面功能新增：`{base}/pages/{slug}`，lastmod=updated_at，
    按 sort_order ASC 排在文章之后）、
    标签索引 `{base}/tags`、分类索引 `{base}/categories`、归档 `{base}/archive`

站点设置（2026-10-03 新增）：
- 存储：`settings` 表（key-value：`name` 主键 / `value` / `updated_at`，SQLite/MySQL 共用 SQL，Any 驱动）；
  时间戳沿用全库 RFC3339 UTC 文本惯例
- 字段（键名）：`site_title`（站点名称）、`site_subtitle`（副标题/口号）、`site_description`
  （meta 描述）、`icp_number`（ICP 备案号，选填）、`footer_text`（页脚自定义文字，选填）、
  `per_page`（每页文章数，默认 10）、`base_url`（RSS/sitemap 绝对 URL 覆盖，选填）
- 默认值：**安装时写入**——title/subtitle 取安装请求 `site.title`/`site.subtitle`，
  base_url 初始值取 config.toml `[server] base_url`（可为空），per_page=10，其余为空串；
  升级安装（表存在但无行）时按键回退同款默认值（title/subtitle 回退 config.toml `[site]`）
- 生效方式：进程内实时（读取路径每次请求查库），**修改后无需重启**
- `GET /api/site/settings` → `SiteSettingsPublic`（公开、无需鉴权；供前台头部/页脚/分页默认值渲染）
  - **不含 base_url 等敏感字段**；不进未安装门禁白名单，未安装 → 503 `not_installed`
- `GET /api/admin/site/settings`（Bearer）→ `SiteSettingsAdmin`（全部字段，含 base_url）
- `PUT /api/admin/site/settings`（Bearer）→ 200 `SiteSettingsAdmin`（更新后的完整设置）
  - body: `{title, subtitle?, description?, icp_number?, footer_text?, per_page, base_url?}`
    （可选字段缺失/null 视为空串；全量更新语义）
  - 校验（失败 → 422 `validation_error`）：
    - title trim 后非空，≤255 字符；subtitle ≤255；description ≤1000；icp_number ≤100；
      footer_text ≤1000；base_url ≤500
    - per_page 整数 1~100
    - base_url 非空时必须是合法绝对 URL 且 scheme 为 http/https
  - 未登录/token 无效 → 401 `unauthorized`
- 联动读取（改为读站点设置而非硬编码/config.toml）：
  - `GET /api/site` 的 title/subtitle
  - `GET /api/posts`、`GET /api/search` 未传 per_page 时的默认值
  - `GET /api/feed.xml` 的 channel title/description、feed 与 sitemap 的绝对 URL（优先级见「RSS 与 sitemap」）

页面（2026-10-03 新增）：

页面（Page）是站点级单页实体，区别于文章：带自己的 slug、启用状态与内容，
用于「关于」「留言板」「友情链接」及管理员自建页。

- 存储：`pages` 表（id、title、slug 唯一、kind、content_md、content_html、enabled、
  sort_order、built_in、created_at/updated_at；时间戳沿用全库 RFC3339 UTC 文本惯例）
  + `page_links` 表（友情链接：page_id、name、url、description、sort_order）。
  SQLite/MySQL 共用 SQL（Any 驱动，`?` 占位符，禁单方言）
- kind 取值：`custom`（普通页）、`message_board`（留言板：前台页尾挂留言表单）、
  `links`（友情链接：前台页尾渲染链接卡片列表）。**kind 创建后不可改**（接口不接受该字段）
- content_html 由后端 pulldown-cmark 渲染（与文章同款管线），保存时写库；
  公开详情接口按文章同款钩子管线**实时**产出：post.before_render 改写 content_md →
  Markdown 渲染 → post.after_render 改写 content_html（复用现有文章钩子，页面不新增钩子）
- 内置页面：安装时自动注入 3 个 built_in=1 的页面（关于 slug=about/kind=custom、
  留言板 slug=guestbook/kind=message_board、友情链接 slug=links/kind=links，
  sort_order 依次 10/20/30，友情链接附示例链接）；启动路径按 slug 幂等补齐缺失的内置页。
  **built_in 页面不可删除（422 `page_builtin`），可停用、可改标题/内容/slug/排序/链接**

公开接口（已安装后可用；**不进未安装门禁白名单**，未安装 → 503 `not_installed`）：
- `GET /api/pages` → `[PageSummary]`，仅 enabled，按 sort_order ASC, id ASC；
  同时是前台顶栏导航「首页 + 启用页面」的数据源
- `GET /api/pages/:slug` → `PageDetail`；不存在/**停用** → 404 `not_found`；
  kind=links 时 links 为该页链接（sort_order ASC, id ASC），其余 kind 恒为 `[]`
- `GET /api/pages/:slug/comments` → `[CommentPub]`（仅 approved，时间 ASC）
- `POST /api/pages/:slug/comments` → 201 `CommentPub`，body 与文章评论相同
  `{author_name, email?, content}`；必填校验 422 `validation_error`；先发后审（创建即 approved）；
  comment.before_create 钩子链同样生效（ctx.post_slug = 页面 slug）
- 留言/页面评论仅限 kind=message_board 的启用页面；其余 kind 或停用页 → 404 `not_found`

留言与评论模型的整合（对现有契约破坏最小的方案）：
- `comments` 表新增 `target_type` 列（`'post'`|`'page'`，默认 `'post'`，旧数据自动视为文章评论）；
  **复用现有 `post_id` 列作为通用目标 id**（target_type='page' 时存页面 id），不新增 target_id 列
- `CommentAdmin` 响应新增 `target_type` 字段；`post_id`/`post_title` 字段名保留、语义扩展为
  「目标 id / 目标标题」（页面留言的 post_title = 页面标题），旧前端读取不受破坏
- 文章评论接口（`/api/posts/:slug/comments`）行为完全不变

管理接口（全部需要 Bearer）：
- `GET /api/admin/pages` → `[PageAdmin]`（含停用页），按 sort_order ASC, id ASC
- `GET /api/admin/pages/:id` → `PageAdmin`；不存在 → 404
- `POST /api/admin/pages` → 201 `PageAdmin`（创建自定义页面，kind 恒为 custom）
  - body: `{title, slug?, content_md, enabled?, sort_order?, links?}`
  - slug 为空时自动生成（ASCII slugify；纯中文标题回退 `page-<id>`，插入后回填）；
    slug 在 pages 表内唯一，冲突 → 409 `slug_taken`（与文章 slug 各自独立命名空间）
  - title trim 后非空 ≤255；slug ≤255；content_md 必填（可为空串）；校验失败 → 422 `validation_error`
  - 自定义页 kind=custom，links 字段忽略（恒为 `[]`）
- `PUT /api/admin/pages/:id` → `PageAdmin`（字段可选更新；**kind 不可改**，请求体中不接受）
  - body: `{title?, slug?, content_md?, enabled?, sort_order?, links?}`
  - links 为**全量替换**语义（按数组顺序重写 sort_order）：仅 kind=links 页面接受，其余 kind 忽略；
    单条校验：name/url trim 后非空（name ≤100、url ≤500 且必须为 http/https 绝对 URL、
    description ≤500），失败 → 422 `validation_error`
- `PATCH /api/admin/pages/:id/toggle` → `PageAdmin`（enabled 取反；停用后前台立即 404、导航消失）
- `DELETE /api/admin/pages/:id` → 204（连带删除该页 page_links 与 target_type='page' 的留言）；
  built_in → 422 `page_builtin`
- 后台留言列表（`GET /api/admin/comments`）返回 `target_type` 供后台区分来源文章/页面

主题设置（2026-10-03 新增；完整规格见 docs/extensibility-contract.md「主题设置项」）：
- 每个主题可在 theme.toml 用 `[[settings]]` 数组自描述可配置项
  （key/label/type/group/default/options；type ∈ text|textarea|color|select|switch|number）；
  值按主题 slug 存 `theme_settings` 表（theme_slug, key, value, updated_at），
  切换主题不丢、**删除主题连带删除其设置**
- `ThemeSetting = {key, label, type, group?, default?, options?: [{value, label}]}`
- `GET /api/themes/:slug/settings`（公开；未安装门禁白名单，未安装时 values=声明默认值）
  → 200 `{slug, settings: [ThemeSetting], values: {key: string|number|bool}}`
  （values 为声明 default 与已存值合并后的生效值，按类型输出）；slug 不存在 → 404
- `GET /api/admin/themes/active/settings-panel`（Bearer）→ 200 `{slug, name, settings, values}`
  （仅服务当前激活主题，后台设置面板数据源）
- `PUT /api/admin/themes/:slug/settings`（Bearer）body `{values: {key: value, ...}}`
  → 200 合并后的最新 `{slug, settings, values}`；部分更新语义（仅写入出现的 key）
  - key 未在声明内 → 422 `unknown_setting`；值类型不符/select 越界/color 非 hex/
    超长（text ≤500、textarea ≤5000 字符）→ 422 `invalid_value`；slug 未安装 → 404
- 前端应用：生效值写入 `:root` 的 `--theme-setting-<key（_ 换 -）>` CSS 变量与
  `data-setting-*` 属性；layout 生效值写 `data-layout`；切换主题/保存设置后
  重新拉取即时生效，无需刷新

## 鉴权
- `POST /api/auth/login` body `{username, password}` → 200 `AuthResult`；错误 → 401 `invalid_credentials`
- `GET /api/auth/me`（Bearer）→ `{"username"}`；无效/过期 → 401 `unauthorized`
- JWT HS256，有效期 7 天，secret 来自 config.toml

## 管理接口（全部需要 Bearer）
文章：
- `GET /api/admin/posts?status=<draft|published|all>&page&per_page` → 分页 `[PostAdmin]`，updated_at DESC
- `POST /api/admin/posts` → 201 `PostAdmin`
  - body: `{title, slug?, content_md, excerpt?, category_id?, tag_ids?: [int], status}`
  - slug 为空时自动生成（ASCII slugify；纯中文标题则回退 `post-<id>`，插入后回填）；slug 唯一，冲突返回 409 `slug_taken`
  - status=published 且首次发布时写 published_at
- `GET /api/admin/posts/:id` → `PostAdmin`
- `PUT /api/admin/posts/:id` → `PostAdmin`（同 POST body，字段可选更新；draft→published 时若 published_at 为空则写入）
- `DELETE /api/admin/posts/:id` → 204

分类/标签：
- `GET /api/admin/categories` → `[Category]`；`POST` body `{name}` → 201；`PUT /:id`；`DELETE /:id` → 204（分类下有文章时 409 `in_use`）
- `GET /api/admin/tags` → `[Tag]`；`POST` body `{name}` → 201；`PUT /:id`；`DELETE /:id` → 204（同上 `in_use`）
- 名称唯一，重复 → 409 `duplicate_name`

评论（2026-10-03 页面功能扩展：列表含页面留言，响应带 `target_type` 区分来源文章/页面，
`post_id`/`post_title` 语义扩展为「目标 id / 目标标题」，见「页面」）：
- `GET /api/admin/comments?status=<approved|hidden|all>&post_id=&page&per_page` → 分页 `[CommentAdmin]`，created_at DESC
  - `post_id` 过滤仅匹配来源为文章的评论（target_type='post' 且目标 id 相等）
- `PUT /api/admin/comments/:id` body `{status}` → `CommentAdmin`
- `DELETE /api/admin/comments/:id` → 204

图片上传（2026-10-03 新增）：
- `POST /api/admin/uploads`（multipart/form-data，字段名 `file`）→ 200 `UploadResult`
  - 允许类型：png、jpeg、gif、webp；**按文件头 magic bytes 判定真实类型**，不信任扩展名与
    Content-Type；svg 一律拒绝（XSS 风险）。类型不符 → 422 `invalid_file_type`
  - 大小上限 config.toml `[uploads] max_size_mb`（默认 10MB），超出 → 422 `file_too_large`
  - 存储：后端运行目录 `uploads/<yyyy>/<mm>/<sha256 前 16 位 hex>.<规范扩展名>`（扩展名由
    判定出的真实类型决定：png/jpg/gif/webp）；文件名不含用户输入，天然防穿越；不入数据库
  - **内容哈希去重**：同 sha256 的文件已存在（任意年月目录）则直接返回已有 url，不重复落盘
  - `filename` 仅回显原始文件名（JSON 转义），未提供时回退生成的文件名
  - 请求体上限 = `max_size_mb` + 1MB multipart 开销余量（超出上限的文件仍报 422 `file_too_large`）

## CORS
- 后端允许来源：`http://localhost:5173`（可在 config.toml `[cors] allowed_origins` 配置，默认含此项）；允许方法 GET/POST/PUT/PATCH/DELETE/OPTIONS（PATCH 为 2026-10-03 页面 toggle 接口新增），允许头 Authorization/Content-Type

## 端口约定
- 后端：3000（config.toml 可改）；前端 dev server：5173（Vite 默认），`/api` 代理到后端
