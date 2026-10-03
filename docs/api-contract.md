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
                tags: [{id, name}], published_at, comment_count,
                is_sticky: bool,   // 2026-10-03 置顶新增，见「文章置顶与定时发布」
                view_count: int, likes: int}
               // 2026-10-04 浏览量与点赞新增：view_count=浏览量（计数与去重策略见
               // 「浏览量与点赞」）；likes=点赞总数（post_likes 子查询计数）
PostDetail   = PostPublic + {content_md,
                prev_post: {title, slug}|null, next_post: {title, slug}|null}
               // 2026-10-04 上一篇/下一篇新增：prev=发布时间更早、next=更晚的相邻文章，
               // 纯时间序、不受置顶影响；仅详情接口有这两个字段（见「文章上一篇/下一篇」）
SearchResult = PostPublic + {snippet}   // snippet 为纯文本上下文片段，见「全文搜索」
PostAdmin    = {id, title, slug, content_md, excerpt,
                status: "draft"|"published"|"scheduled",  // scheduled 为 2026-10-03 定时发布新增
                category_id|null, category_name|null, tag_ids: [int],
                published_at|null,   // status=scheduled 时即计划发布时间（RFC3339 UTC）
                is_sticky: bool,
                view_count: int, likes: int,  // 2026-10-04 新增（后台只读展示）
                created_at, updated_at}
Category     = {id, name, post_count}
Tag          = {id, name, post_count}
CommentPub   = {id, author_name, content, created_at,
                parent_id|null, reply_to_id|null, reply_to_name|null}
               // 2026-10-03 嵌套评论新增：parent_id=所属顶级楼层 id（顶级评论本身为 null）；
               // reply_to_id=被回复的中间楼层 id（仅「回复的回复」非 null）；
               // reply_to_name=被回复人作者名（JOIN 冗余，reply_to_id 为 null 时同为 null）
CommentAdmin = {id, post_id, post_title, author_name, email|null, content,
                status: "approved"|"hidden", created_at,
                target_type: "post"|"page",   // 2026-10-03 页面功能扩展，见「页面」
                parent_id|null, reply_to_id|null, reply_to_name|null,
                reply_count: int}             // 2026-10-03 嵌套评论新增：直接子回复条数
                                              // （两级存储下即整线程楼层数；子回复恒 0）
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
SiteStats      = {post_count: int, comment_count: int, installed_at: string,
                  total_views: int}
               // 2026-10-03 组件系统新增：published 文章数 / approved 评论数（含页面留言）/
               // 安装时间（users 表最早 created_at，RFC3339；取不到时为空串）——站点信息组件数据源
               // 2026-10-04 浏览量新增：total_views = 所有文章 view_count 之和（posts 全表
               // SUM，含草稿——历史累计口径），见「浏览量与点赞」
WidgetPosition = "sidebar" | "left" | "right" | "footer"   // 规范位置枚举（校验范围）
WidgetKind     = "builtin" | "custom"                      // builtin=前端内置 React 组件；custom=HTML 片段
WidgetSource   = "builtin" | "theme" | "admin"             // 组件来源：内置 / 主题声明 / 后台自建
WidgetConfig   = {key, kind: WidgetKind, label, position: WidgetPosition,
                  sort_order: int, config: {<参数键>: string|number|bool}}
               // 公开形状；custom 组件的 config 含 title（可空）与 html（已做 {{param}} 替换）
WidgetAdmin    = WidgetConfig + {source: WidgetSource, enabled: bool, params: [ThemeSetting]}
               // 管理形状；params 为该组件可配置参数声明（复用 ThemeSetting 形状，见「主题设置」）
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

**公开可见性（2026-10-03 定时发布新增，惰性发布方案）**：所有公开查询（文章列表/详情/标签/
分类/归档/搜索/RSS/sitemap/评论目标可见性）中文章的可见条件统一为
`(status='published' OR (status='scheduled' AND published_at <= :now))`——scheduled 文章
到点自动可见，**无后台定时器、无需重启**；`:now` 由后端在每次请求时以 RFC3339 UTC 字符串
传入（全库时间戳字典序即时间序，SQLite/MySQL 共用同一份 SQL）。计数类逻辑（分类/标签
post_count、站点统计 SiteStats.post_count、归档计数）同口径。详见「文章置顶与定时发布」。

- `GET /api/site` → `SiteInfo`（title/subtitle 与站点设置一致，见「站点设置」）
- `GET /api/posts?page&per_page&tag=<name>&category=<name>&year=<int>&month=<int>&order=<recent|hot>` → 分页 `[PostPublic]`，仅公开可见（见上）；列表不含 content_md；**列表接口不产生浏览量计数**（见「浏览量与点赞」）
  - order（2026-10-03 组件系统新增；2026-10-03 置顶调整；2026-10-04 hot 改为浏览量优先）：
    `recent`（默认，缺省即此）按 **is_sticky DESC, published_at DESC**（置顶在前；
    tag/category/year/month 过滤后的标签/分类/归档列表同此规则）；`hot` 按
    **view_count DESC, comment_count DESC, published_at DESC**（热门文章组件数据源；
    comment_count 为既有 SELECT 别名，SQLite/MySQL 均支持按别名排序；
    **不受置顶影响**）；其他值 → 422 `validation_error`
- excerpt 为空时的回退（2026-10-03 定）：由 content_md 生成**纯文本**摘要（剥离 Markdown 语法：标题#、强调符、代码围栏、表格线、链接保留文字），截断至 ≤200 字符；不得返回含 Markdown 符号的原文
- `GET /api/posts/:slug` → `PostDetail`；不存在/未公开可见（草稿、未到点的 scheduled）→ 404 `not_found`
  - **上一篇/下一篇**（2026-10-04 新增，见「文章上一篇/下一篇」）：`prev_post`/`next_post` 为
    `{title, slug}|null`；prev = 发布时间更早的相邻文章、next = 更晚，纯时间序、**不受置顶影响**；
    草稿与未到点的 scheduled 不作为相邻项出现
  - **浏览量计数点**：每次公开命中 view_count + 1（去重/排除规则见「浏览量与点赞」）；
    计数成功时响应中的 view_count 已含本次
- `GET /api/posts/:slug/like?liker_key=` → 200 `{likes: int, liked: bool}`（当前访客是否已赞，
  前端进详情页时调用决定按钮初始状态；见「浏览量与点赞」）
- `POST /api/posts/:slug/like` → 200 `{likes: <新总数>, liked: true}`，body `{liker_key}`（见「浏览量与点赞」）
- `DELETE /api/posts/:slug/like` → 200 `{likes, liked: false}`，liker_key 走 query 或 JSON body（见「浏览量与点赞」）
- `GET /api/posts/:slug/comments` → `[CommentPub]`（仅 approved 且线程可见，按时间 ASC, id ASC；
  仍为平铺数组，两级树由前端按 parent_id 自行组装，见「评论回复」）
  - 评论目标可见性同文章：文章未公开可见（含未到点的 scheduled）→ 404 `not_found`
- `POST /api/posts/:slug/comments` → 201 `CommentPub`
  - body: `{author_name, email?, content, parent_id?}`；必填校验 422 `validation_error`
  - 默认先发后审：创建即 approved
  - 带 parent_id 时为回复（楼中楼）：校验与两级归一化见「评论回复」
- `GET /api/tags` → `[Tag]`（post_count 只统计公开可见文章，见「公开可见性」）
- `GET /api/categories` → `[Category]`（同上）
- `GET /api/archive` → `[{"year": int, "month": int, "count": int}]`，仅公开可见文章，按年月 DESC
  （scheduled 文章到点后按计划时间所在年月计入）

全文搜索（2026-10-03 新增）：
- `GET /api/search?q=<关键词>&page&per_page` → 分页 `[SearchResult]`（复用总则分页壳），
  仅公开可见文章，按 published_at DESC（**不受置顶影响**）；分页参数与 `/api/posts` 相同
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
  - 最新 20 篇公开可见文章（含到点的 scheduled），按 published_at DESC——
    **保持纯时间序，不受置顶影响**（2026-10-03 置顶条款）
  - channel 含 title、link（站点绝对 URL）、description（来源见上）
  - item 含 title、link（`{base}/posts/{slug}`，与前端路由一致）、guid（isPermaLink=true，同 link）、
    pubDate（RFC 822）、description（excerpt，为空时按「excerpt 回退」条款从正文推导）
  - 所有文本 XML 转义（`& < > " '`）
- `GET /api/sitemap.xml` → urlset（xmlns `http://www.sitemaps.org/schemas/sitemap/0.9`），
  Content-Type `application/xml; charset=utf-8`
  - 含：首页 `{base}/`、全部公开可见文章详情页（含到点的 scheduled；lastmod=updated_at，
    W3C datetime 即 RFC3339；**保持纯时间序，不受置顶影响**）、
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
- `GET /api/site/stats` → `SiteStats`（2026-10-03 组件系统新增；公开、无需鉴权，
  站点信息组件数据源。不进未安装门禁白名单，未安装 → 503 `not_installed`）

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
- `GET /api/pages/:slug/comments` → `[CommentPub]`（仅 approved 且线程可见，时间 ASC, id ASC；
  形状与线程规则同文章评论，见「评论回复」）
- `POST /api/pages/:slug/comments` → 201 `CommentPub`，body 与文章评论相同
  `{author_name, email?, content, parent_id?}`；必填校验 422 `validation_error`；先发后审（创建即 approved）；
  comment.before_create 钩子链同样生效（ctx.post_slug = 页面 slug）；
  回复（parent_id）与文章评论同一套机制：校验/两级归一化/连带删除/隐藏线程过滤见「评论回复」
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

主题组件（widgets，2026-10-03 新增；声明规格与自定义组件写法见
docs/extensibility-contract.md「主题组件」与 docs/theme-development.md「组件（widgets）」）：

- 存储：**独立 `theme_widgets` 表**（按主题 slug 隔离，与 theme_settings 同款生命周期：
  切换主题不丢、删除主题连带删除其行）。选独立表而非复用 theme_settings 的原因：
  组件配置是**多行结构化记录**（每组件一行：enabled/position/sort_order/config JSON），
  塞进 key-value 表需要序列化整表且无法按行校验，独立表契约更清晰。
  SQLite/MySQL 共用 SQL（Any 驱动，`?` 占位符）；时间戳沿用全库 RFC3339 UTC 文本惯例
- 组件全集 = **内置组件**（后端注册表 + 前端 React 实现，共 7 个：`recent-posts` 最新文章、
  `hot-posts` 热门文章、`tag-cloud` 标签云、`categories` 分类列表、`archive` 归档、
  `links` 友情链接、`site-info` 站点信息）+ **主题声明组件**（theme.toml `[[widgets]]`，
  kind=custom，HTML 片段来自 `assets/widgets/<key>.html`）+ **后台自建自定义组件**
  （kind=custom、source=admin，HTML 存 config.html）。每个组件的参数 schema 见
  `WidgetAdmin.params`（内置组件参数：title 标题文字、count 显示条数等）
- **默认值**（安装即生效、无需写库；DB 行仅是覆盖）：default 主题默认启用
  `recent-posts`(sort 10)、`tag-cloud`(sort 30)、`categories`(sort 40)，位置均 sidebar；
  其余内置组件默认停用。默认启用集与主题无关（任何主题的内置组件默认值一致）
- position 规范枚举恒为 `sidebar|left|right|footer` 四值（存储与校验与布局无关）；
  **布局降级映射由前端执行**：双列布局（topbar-two-column）可用区域为 sidebar/footer，
  left/right 降级并入 sidebar 列；三列布局（topbar-minimal-three-column）可用区域为
  left/right/footer，sidebar 降级映射到右栏。某区域无启用组件时不渲染空壳
- 公开接口（未安装门禁**白名单**，未安装时返回内置默认值，保证安装页可渲染）：
  - `GET /api/themes/:slug/widgets` → 200 `{slug, widgets: [WidgetConfig]}`
    —— 仅 **enabled** 组件，按 sort_order ASC, key ASC；custom 组件的 config.html
    已完成 `{{param}}` 令牌替换（值取生效 config，不转义——与插件注入同信任模型）；
    slug 磁盘不存在 → 404（default 缺盘以内置常量兜底，同主题设置）
- 管理接口（均需 Bearer；按 slug 读写，slug 未安装 → 404）：
  - `GET /api/admin/themes/:slug/widgets` → 200
    `{slug, positions: [WidgetPosition], widgets: [WidgetAdmin]}`
    —— widgets 为**全量合并列表**：内置注册表 + 主题声明组件（默认值打底、已存行覆盖）
    + 自建 custom 行，按 sort_order ASC, key ASC；positions 为规范枚举（校验范围）
  - `PUT /api/admin/themes/:slug/widgets` body `{widgets: [WidgetInput]}` → 200 同 GET 形状
    - `WidgetInput = {key, kind: "builtin"|"custom", enabled?, position, sort_order?, config?}`
      （enabled 缺省 false、sort_order 缺省 0、config 缺省 {}）
    - **全量替换语义**：请求数组即该主题最终配置；未出现的内置/主题组件的已存行删除
      （回到默认值），未出现的自建 custom 组件删除（即「删组件」）
    - 校验（失败 → 422，均不写库）：widgets 非数组 → `validation_error`；
      key 重复 → `invalid_value`；kind=builtin 但 key 不在内置注册表 → `unknown_widget`；
      kind=custom 的 key 非法（不匹配 ^[a-z0-9][a-z0-9_-]{0,63}$）或与内置 key 冲突 →
      `invalid_value`；position 不在规范枚举 → `invalid_value`；config 非对象/含未声明参数/
      参数类型不符/超长（title ≤200 字符、html ≤65536 字符、其余参数按 params 声明的
      text ≤500 / textarea ≤5000 规则）→ `invalid_value`；单主题组件数 >100 → `invalid_value`
    - 主题声明组件（source=theme）的 html 权威来源是主题包 `assets/widgets/<key>.html`
      （每次读取时从磁盘加载）；config.html 非空时作为**后台覆盖**优先于文件
- 生效方式：前台按 position+sort_order 渲染；后台保存后前端 store 重新拉取即时生效，
  无需刷新

## 评论回复（楼中楼，2026-10-03 新增）

文章评论与留言板留言共用同一套回复机制（仅 target_type 不同，字段与校验规则完全一致）。

- 存储：`comments` 表新增 `parent_id`、`reply_to_id` 两个可空整数列（SQLite/MySQL 各一份
  migration，ADD COLUMN 双方言均可；SQLite 的 ADD COLUMN 不支持内联自引用 FK，
  自引用一致性由应用层校验保证）
- **两级归一化（Typecho/WP 风格）**：存储上永远两级——
  - `parent_id` 恒指向线程的**顶级祖先**（楼层）：直接回复顶级评论时 parent_id=该评论 id、
    reply_to_id=null；回复某条中间楼层时，新评论的 parent_id 被**改写为**该楼层的顶级祖先 id，
    同时 `reply_to_id`=实际被回复的楼层 id（显示层据此渲染「回复 @某人」）
  - 归一化在写入前完成；comment.before_create 钩子拿到的即是归一化后的值
- POST body（文章/留言板两处相同）新增可选 `parent_id`（缺省即顶级评论）。
  校验（失败 → 422 `validation_error`，带明确 message）：
  - 父评论必须存在（「父评论不存在」）
  - 父评论必须与本次请求同目标：target_type 与目标 id 均一致（「不能回复其他目标下的评论」）
  - 父评论 status 必须为 approved（「不能回复已隐藏的评论」）
- comment.before_create 钩子对回复同样生效：入参 ctx 新增 `parent_id`、`reply_to_id`
  （INT，0 表示无；见 docs/extensibility-contract.md）；block 同样短路 → 403 `comment_blocked`
- 公开列表（`GET /api/posts/:slug/comments` 与 `GET /api/pages/:slug/comments`）：
  仍返回**平铺数组**（仅 approved，按 created_at ASC, id ASC），每项新增
  parent_id/reply_to_id/reply_to_name（reply_to_name 为 JOIN 取到的被回复人作者名，
  reply_to_id 为 null 时同为 null）；前端按 parent_id 自行组两级树
- **隐藏线程过滤**：顶级评论被 hidden 时整条线程不出现在公开列表
  （公开查询把 parent_id 指向非 approved 评论的子回复一并排除）；子回复自身 status
  不连带变更——父恢复 approved 后线程整体重新可见
- **comment_count**（文章列表/详情）：口径与公开列表一致——只统计前台可见的评论
  （approved 且线程可见），线程内所有 visible 评论都计数
- 管理端：
  - `CommentAdmin` 新增 parent_id/reply_to_id/reply_to_name/reply_count
    （reply_count=直接子回复条数，两级存储下即整线程楼层数，供后台提示
    「删除将连带删除 N 条回复」；子回复的 reply_count 恒为 0）
  - 列表排序不变（created_at DESC），父子回复在列表中不强制相邻，线程关系由字段表达
  - **连带删除**：`DELETE /api/admin/comments/:id` 删除顶级评论时连带删除其全部子回复
    （仍 204）；删除子回复只删自身；id 不存在 → 404 `not_found`
  - 隐藏顶级评论（`PUT` status=hidden）不改子回复 status，仅前台整线程过滤

## 文章置顶与定时发布（2026-10-03 新增）

### 文章置顶（is_sticky）
- 存储：`posts` 表新增 `is_sticky` 整数列（0/1，默认 0；SQLite/MySQL 各一份 migration，
  ADD COLUMN 双方言均可）。旧行自动为 0，对已有安装幂等
- 形状：`PostPublic`/`PostDetail`/`SearchResult`/`PostAdmin` 均新增 `is_sticky: bool`；
  前台列表摘要卡对置顶文章显示「置顶」徽章（前端渲染约定）
- 排序范围：`GET /api/posts` 的 `recent` 序（默认序）改为 **is_sticky DESC, published_at DESC**
  ——tag/category/year/month 过滤（标签页/分类页/归档月份页共用同一接口）同此规则。
  **不受置顶影响、保持原有排序的**：`order=hot`（view_count DESC, comment_count DESC,
  published_at DESC——2026-10-04 起浏览量优先，见「浏览量与点赞」）、`GET /api/search`
  （published_at DESC）、RSS 与 sitemap（纯时间序 published_at DESC）
- 管理端：
  - `PATCH /api/admin/posts/:id/sticky` body `{is_sticky: bool}` → `PostAdmin`（行内快捷切换；
    选独立 PATCH 而非并入 PUT：与页面 `PATCH /:id/toggle` 同风格，避免后台列表快捷操作
    携带全量 PUT body，对契约破坏最小）
  - `POST`/`PUT /api/admin/posts` body 亦接受可选 `is_sticky`（编辑器置顶开关；缺省：
    POST=false、PUT=保持原值）
  - 置顶不校验文章状态（草稿/定时文章也可先置顶；未公开可见的文章本来就不出现在公开列表，
    置顶仅在文章公开可见后产生排序效果）
  - sticky 切换会更新 updated_at（与其他管理写操作一致）

### 定时发布（status=scheduled，惰性发布方案）
- status 枚举新增 `scheduled`（status 列为 TEXT，**无需迁移改表**）；
  `PostAdmin.published_at` 在 scheduled 状态下即计划发布时间（RFC3339 UTC）
- **惰性可见性（不引入后台定时器）**：所有公开查询（列表/详情/标签/分类/归档/搜索/RSS/
  sitemap/评论目标可见性）的可见条件从 `status='published'` 改为
  `(status='published' OR (status='scheduled' AND published_at <= :now))`；`:now` 由 Rust 侧
  每次请求以 RFC3339 UTC 字符串绑定传入（字典序即时间序，SQLite/MySQL 共用一份 SQL；
  published_at 为 NULL 时比较结果为 NULL，自然排除）。到点自动可见，**零后台任务、无需重启**
- 计数类逻辑同口径采用可见性条件：分类/标签 post_count（公开与 admin 列表/改名响应）、
  站点统计 `SiteStats.post_count`、归档计数
- 状态到达后 status 仍保持 `scheduled`（无后台任务改写），公开可见性完全由查询条件决定；
  后台列表状态列显示「定时发布」+ 计划时间（前端渲染约定）
- 校验（失败 → 422 `validation_error`，带明确 message）：
  - POST/PUT 使 status 变为 `scheduled`（或 PUT 显式提供 published_at 改期）时：
    published_at 必填、必须为合法 RFC3339、且必须晚于当前时间（「请使用未来时间」）；
    写入前归一化为 UTC 秒精度（如 `2026-10-03T12:00:00Z`，前端本地时区输入自行转 UTC）
  - **编辑已到点的 scheduled 文章**（不改 status、不提供 published_at，如只改标题/正文）
    不重新校验未来时间——否则惰性可见后文章将永远无法编辑
  - body 的 `published_at` 字段仅在 status=scheduled 时被接受；status=published/draft 时忽略
- 状态转换：
  - `scheduled → published`：允许（**立即发布**），published_at 改写为当前时间，
    触发 post.after_publish 钩子
  - `published → scheduled`：拒绝 → 422 `validation_error`（「已发布文章不能改为定时发布，
    请先转为草稿」）
  - `scheduled → draft`：允许（取消定时），published_at 清空为 null
  - `draft → published`：现有行为不变（published_at 为空则写入当前时间）
- 管理接口：`GET /api/admin/posts?status=scheduled` 过滤定时文章；`all` 含 scheduled
- 钩子说明（扩展契约）：post.after_publish 仅在**显式发布动作**（创建即发布、
  draft→published、scheduled→published 手动切换）时触发；惰性到点可见**不触发**钩子
  （无后台任务），此为惰性方案的固有取舍
- RSS/sitemap：到点的 scheduled 文章正常进入（可见性条件），排序保持纯时间序不受置顶影响

## 浏览量与点赞（2026-10-04 新增）

### 浏览量（view_count）
- 存储：`posts` 表新增 `view_count` 整数列（NOT NULL DEFAULT 0；SQLite/MySQL 各一份
  migration，对已有安装幂等，旧行自动为 0）
- **计数点：仅公开 `GET /api/posts/:slug`（文章详情）**，每次命中执行
  `UPDATE posts SET view_count = view_count + 1`（自增写法双方言通用）；
  列表/搜索/feed/sitemap 等其他接口一律不计数。计数成功时详情响应中的
  view_count 已含本次
- 不计数的情形：
  - 带 Bearer 凭据的请求（后台管理端预览；只认 `Authorization: Bearer <非空>` 头部
    存在性，不校验 token 有效性）
  - 爬虫/自动化 UA（常见关键字小写子串匹配：bot、crawl、spider、slurp、curl、wget、
    python、httpclient、okhttp、headless、scanner、preview、facebookexternalhit、
    mediapartners、feedfetcher、lighthouse）
- **去重（尽力而为，非精确审计）**：进程内存表记 `(ip, post_id) → 最近计数时刻`，
  **60 分钟窗口内同一来源对同一文章只计一次**；内存态、重启清零，不引入 Redis 等
  外部依赖；表条目达到软上限（50000）时整表清扫过期项
  - ip 取值优先级：`X-Forwarded-For` 首项 → `X-Real-IP` → TCP 直连地址（生产路径
    提供 ConnectInfo）→ 兜底 `"direct"`（无任何来源信息时视为同一来源——个人博客
    反代部署下 XFF 恒在，直连裸奔场景从简合并，符合「尽力去重」定位）
- 消费点：
  - `PostPublic`/`PostDetail`/`PostAdmin` 形状新增 `view_count`（前台列表摘要卡与
    详情页眼睛图标显示；后台只读展示）
  - `order=hot` 排序改为 **view_count DESC, comment_count DESC, published_at DESC**
    （热门文章组件数据源；副标题显示浏览数）
  - `GET /api/site/stats` 新增 `total_views`：所有文章 view_count 之和（posts 全表
    SUM，含草稿——历史累计口径；站点信息卡显示）

### 点赞（post_likes）
- 存储：新表 `post_likes`（id、post_id、liker_key、created_at；SQLite/MySQL 各一份
  migration，时间戳沿用全库 RFC3339 UTC 文本惯例）；**(post_id, liker_key) UNIQUE**
  （MySQL 用 UNIQUE KEY、SQLite 用 UNIQUE，各自 migration 内合法；该索引最左前缀
  同时服务按 post_id 的计数子查询）
- **liker_key = 前端匿名 id**：前端首次生成 UUID 存 localStorage `reedblog_like_id`，
  点赞三接口都带上；后端只校验「trim 后非空且 ≤64 字符」，不做真伪验证
  （匿名点赞天然可换浏览器/清存储重刷，定位为轻量互动而非精确民意）
- 接口（公开、无需鉴权；**不进未安装门禁白名单**，未安装 → 503 `not_installed`）：
  - `POST /api/posts/:slug/like` body `{liker_key}` → 200 `{likes: <新总数>, liked: true}`；
    重复点赞同 key → **幂等**返回当前状态（不报错，likes 不涨——UNIQUE 冲突视为已赞）
  - `DELETE /api/posts/:slug/like`，liker_key 走 query（`?liker_key=`）或 JSON body
    （query 优先）→ 200 `{likes, liked: false}`；未点赞过 → **幂等**（删除零行不报错）
  - `GET /api/posts/:slug/like?liker_key=` → 200 `{likes, liked}`（当前访客是否已赞，
    前端进详情页时调用决定按钮初始状态）
  - 校验：liker_key 缺失/trim 后为空/超 64 字符 → 422 `validation_error`；
    文章不存在或未公开可见（草稿、未到点的 scheduled）→ 404 `not_found`（口径同
    评论目标可见性）
- 形状：`PostDetail` 与 `PostPublic` 列表都带 `likes`（点赞总数，post_likes 子查询
  计数——文章量小，双方言性能可接受）；`PostAdmin` 亦含 `view_count`/`likes`
  （后台列表只读展示，**不做管理点赞**）
- 删除文章连带删除其 post_likes 行（`DELETE /api/admin/posts/:id` 清理范围扩展）
- 前端交互约定：详情页点赞按钮（心形图标 + 数字），点击乐观更新 + scale 弹跳动画，
  已赞态高亮，再点取消；localStorage 记住匿名 id

## 文章上一篇/下一篇（2026-10-04 新增）

- 形状：`PostDetail` 新增 `prev_post`/`next_post`，各为 `{title, slug} | null`（只带标题与
  slug，不含正文）。**仅详情接口有这两个字段**：`PostPublic`、列表、搜索、RSS/sitemap 的
  形状与行为完全不变
- 语义：**prev = 发布时间更早的那篇，next = 发布时间更晚的那篇**。相邻关系按**纯发布时间**
  排序 `published_at DESC, id DESC` 取当前篇紧邻的前/后一条（id 做同秒发布的稳定 tiebreak）；
  某方向没有相邻文章时为 `null`（最新一篇 next=null、最老一篇 prev=null）
- **置顶（is_sticky）不影响相邻关系**：相邻排序是纯时间序，不是列表页的 sticky 优先序
  ——置顶会把某篇文章人为拽到所有位置旁边，破坏「上一篇/下一篇」的时间语义
- 可见性谓词与所有公开查询完全一致（见「公开可见性」）：
  `(status='published' OR (status='scheduled' AND published_at <= :now))`——草稿、未到点的
  scheduled 都不作为相邻项出现；已到点的 scheduled 正常参与
- 实现约束：相邻查询在详情 handler 内完成（**不新增独立端点**，避免前端额外往返）；
  只查 `title`/`slug` 两列；SQLite/MySQL 共用一份 SQL——`(published_at, id)` 用显式
  `OR` 展开比较，不用行值比较/窗口函数等方言或版本敏感特性
- 前端渲染约定：详情页正文底部、评论区之前显示「上一篇 / 下一篇」双栏导航
  （左 = 更早、右 = 更晚，显示标题、可点击跳转）；单侧为 `null` 时该侧留空
  （不出现死链接），窄屏（<sm）改为上下堆叠；配色走现有主题 token 与暗色模式

## 鉴权
- `POST /api/auth/login` body `{username, password}` → 200 `AuthResult`；错误 → 401 `invalid_credentials`
- `GET /api/auth/me`（Bearer）→ `{"username"}`；无效/过期 → 401 `unauthorized`
- JWT HS256，有效期 7 天，secret 来自 config.toml

## 管理接口（全部需要 Bearer）
文章（2026-10-03 置顶与定时发布扩展，完整规则见「文章置顶与定时发布」）：
- `GET /api/admin/posts?status=<draft|published|scheduled|all>&page&per_page` → 分页 `[PostAdmin]`，updated_at DESC
  - status 过滤支持 `scheduled`（定时发布）；`all`（缺省）含全部三种状态；非法值 → 422 `validation_error`
- `POST /api/admin/posts` → 201 `PostAdmin`
  - body: `{title, slug?, content_md, excerpt?, category_id?, tag_ids?: [int], status,
    is_sticky?, published_at?}`
  - slug 为空时自动生成（ASCII slugify；纯中文标题则回退 `post-<id>`，插入后回填）；slug 唯一，冲突返回 409 `slug_taken`
  - status=published 且首次发布时写 published_at（body 的 published_at 忽略）
  - status=scheduled 时 published_at 必填且必须是未来时间（RFC3339；写入时归一化为
    UTC 秒精度），否则 → 422 `validation_error`
  - is_sticky 可选（缺省 false）
- `GET /api/admin/posts/:id` → `PostAdmin`
- `PUT /api/admin/posts/:id` → `PostAdmin`（同 POST body，字段可选更新；draft→published 时若
  published_at 为空则写入；scheduled→published 为「立即发布」，published_at 改写为当前时间；
  published→scheduled 拒绝 → 422；scheduled→draft 清空 published_at；其余状态转换规则与
  published_at/is_sticky 语义见「文章置顶与定时发布」）
- `PATCH /api/admin/posts/:id/sticky`（2026-10-03 置顶新增）→ `PostAdmin`
  - body: `{is_sticky: bool}`（必填）；行内快捷置顶/取消置顶，不改其他字段
  - id 不存在 → 404 `not_found`；未登录/token 无效 → 401 `unauthorized`
- `DELETE /api/admin/posts/:id` → 204

分类/标签：
- `GET /api/admin/categories` → `[Category]`；`POST` body `{name}` → 201；`PUT /:id`；`DELETE /:id` → 204（分类下有文章时 409 `in_use`）
- `GET /api/admin/tags` → `[Tag]`；`POST` body `{name}` → 201；`PUT /:id`；`DELETE /:id` → 204（同上 `in_use`）
- 名称唯一，重复 → 409 `duplicate_name`

评论（2026-10-03 页面功能扩展：列表含页面留言，响应带 `target_type` 区分来源文章/页面，
`post_id`/`post_title` 语义扩展为「目标 id / 目标标题」，见「页面」；
2026-10-03 嵌套评论扩展：响应新增 parent_id/reply_to_id/reply_to_name/reply_count，
删除顶级评论连带删除子回复，见「评论回复」）：
- `GET /api/admin/comments?status=<approved|hidden|all>&post_id=&page&per_page` → 分页 `[CommentAdmin]`，created_at DESC（排序不变，父子不强制相邻；时间戳秒精度，同秒行以 id DESC 兜底保证确定性）
  - `post_id` 过滤仅匹配来源为文章的评论（target_type='post' 且目标 id 相等）
- `PUT /api/admin/comments/:id` body `{status}` → `CommentAdmin`
  （隐藏顶级评论时前台整线程过滤，子回复 status 不连带变更）
- `DELETE /api/admin/comments/:id` → 204（顶级评论**连带删除其全部子回复**；子回复只删自身）

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
