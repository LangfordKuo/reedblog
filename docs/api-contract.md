# reedblog API 契约 v1

前后端共同规格。后端（backend/，Rust+Axum）与前端（frontend/，React）都必须以此为准；如需偏离，必须在交付报告里显式列出。

## 总则
- Base URL：`/api`（开发时前端 Vite 代理到 `http://localhost:3000`；生产由 Nginx 反代）
- 全部 JSON，UTF-8。时间戳一律 RFC3339（UTC），如 `2026-10-02T12:00:00Z`
- 错误统一形状：`{"error": {"code": "<snake_case>", "message": "<人类可读>"}}`，配合恰当 HTTP 状态码
- 鉴权：JWT Bearer。请求头 `Authorization: Bearer ***`
- 分页响应统一：`{"items": [...], "total": <int>, "page": <int>, "per_page": <int>}`；分页参数 `page`（默认1）、`per_page`（默认10，上限100）

## 数据形状
```
SiteInfo     = {title, subtitle, installed: bool}
PostPublic   = {id, title, slug, excerpt, category: {id, name}|null,
                tags: [{id, name}], published_at, comment_count}
PostDetail   = PostPublic + {content_md}
PostAdmin    = {id, title, slug, content_md, excerpt, status: "draft"|"published",
                category_id|null, category_name|null, tag_ids: [int],
                published_at|null, created_at, updated_at}
Category     = {id, name, post_count}
Tag          = {id, name, post_count}
CommentPub   = {id, author_name, content, created_at}
CommentAdmin = {id, post_id, post_title, author_name, email|null, content,
                status: "approved"|"hidden", created_at}
AuthResult   = {token, username, expires_at}
```

## 安装向导（未初始化时）
- `GET /api/health` → `{"status": "ok"}`（永远可用）
- `GET /api/install/status` → `{"installed": bool}`（永远可用）
- `POST /api/install` → 201 `{"ok": true}`
  - body: `{db_type: "sqlite"|"mysql", sqlite_path?: string(默认 "reedblog.db"), mysql?: {host, port, username, password, database}, admin: {username, password}, site: {title, subtitle?}}`
  - 行为：验证连接 → 写入 `backend/config.toml` → 建表（按 db_type 跑对应迁移）→ 创建管理员（argon2 哈希）→ 生成 JWT secret 存 config
  - 已安装后再调 → 409 `{"error":{"code":"already_installed",...}}`
- **未安装状态下**，除 `/api/health`、`/api/install/status`、`POST /api/install` 外的所有 `/api/*` 返回 503 `{"error":{"code":"not_installed",...}}`
- 安装完成无需重启进程（进程内切换到已初始化状态即可；实现上允许重启，但接口行为必须一致）

## 站点公开接口（已安装后可用）
- `GET /api/site` → `SiteInfo`
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

评论：
- `GET /api/admin/comments?status=<approved|hidden|all>&post_id=&page&per_page` → 分页 `[CommentAdmin]`，created_at DESC
- `PUT /api/admin/comments/:id` body `{status}` → `CommentAdmin`
- `DELETE /api/admin/comments/:id` → 204

## CORS
- 后端允许来源：`http://localhost:5173`（可在 config.toml `[cors] allowed_origins` 配置，默认含此项）；允许方法 GET/POST/PUT/DELETE/OPTIONS，允许头 Authorization/Content-Type

## 端口约定
- 后端：3000（config.toml 可改）；前端 dev server：5173（Vite 默认），`/api` 代理到后端
