# reedblog

[English](README.en.md) | **中文**

使用 Rust 编写的轻量化博客系统，带插件与主题扩展。
A lightweight blog system written in Rust, with plugin & theme extensibility.

## 特性

**核心博客**

- 安装向导：首次访问通过 Web 界面完成初始化，支持 **SQLite**（零配置）与 **MySQL**
- 文章管理：Markdown 写作（GFM 表格、删除线、任务列表）、草稿/发布两态、slug 自定义或自动生成、摘要缺省时自动从正文提取（纯文本，≤200 字符）
- 分类、标签、按月归档；文章列表分页
- 评论：先发后审（创建即公开），后台可隐藏/恢复/删除
- 图片上传：编辑器工具栏 / 粘贴 / 拖拽即传（PNG/JPEG/GIF/WebP，按 magic bytes 判定真实类型，SVG 拒绝，上限默认 10MB 可配），sha256 内容哈希去重，经 `/api/uploads/*` 静态托管（immutable 强缓存）
- 媒体库：后台上传图片统一入 `media` 表（同内容不重复入库），网格浏览（缩略图/尺寸/大小/时间）、复制 URL、删除（连带删除磁盘文件；删除前提示旧文章引用会 404）；本次改动前的历史文件由列表惰性扫描兜底可见
- RSS 订阅（`/api/feed.xml`，最新 20 篇）与 sitemap（`/api/sitemap.xml`）；站点绝对 URL 支持 `[server] base_url` 配置，为空时按反代头（X-Forwarded-Proto/Host）推导
- 全文搜索：`GET /api/search` 用 LIKE 实现（SQLite/MySQL 双方言零迁移，词条空白切分 AND 语义、通配符转义、仅 published），前端 `/search` 页 URL 携带查询词可分享，命中词 `<mark>` 高亮并显示纯文本上下文 snippet
- 邮件通知（SMTP）：新评论/新回复创建成功后向管理员邮箱发通知邮件（含文章/页面标题与绝对链接、评论者、内容、后台管理链接）；后台「邮件通知」页可配 SMTP（STARTTLS/隐式 TLS/明文）与发送测试邮件。**密码只从 `config.toml` 的 `[smtp] password` 或环境变量 `REEDBLOG_SMTP_PASSWORD` 读取，绝不入库、绝不经接口返回**；发送异步进行（约 10 秒超时，失败只记日志），绝不影响评论发表
- 前台暗色切换：light/dark/system 三态偏好（localStorage 持久化，默认跟随系统），`<head>` 内联脚本在首绘前应用防 FOUC，header sun/moon 一键直切
- 前台：文章代码高亮（highlight.js）、响应式布局
- 后台管理面板：仪表盘、文章、分类、标签、评论、插件、主题

**插件系统**（[开发文档](docs/plugin-development.md)）

- 插件 = 目录 + [Rhai](https://rhai.rs/) 脚本 + manifest.toml，无需编译、无需 Rust 知识，zip 上传即装
- 4 个后端钩子：`post.before_render` / `post.after_render` / `comment.before_create`（可拦截评论）/ `post.after_publish`，多插件按 slug 字典序链式调用
- 前端轻注入：`head` / `body_end` 两个位置的原始 HTML 片段（`<script>` 可执行，适合统计脚本、widget）
- 安全沙箱：无文件/网络/进程能力，限制调用深度、操作数、字符串与集合大小；脚本出错自动跳过并记录 `last_error`
- 热切换：启用/停用/删除立即生效，无需重启

**主题系统**（[开发文档](docs/theme-development.md)）

- 主题 = 设计令牌（theme.toml）+ 自定义 CSS + 静态资源，zip 上传、后台一键激活
- 32 个设计令牌覆盖 shadcn/Tailwind CSS 变量体系（含可选暗色变体 `[tokens_dark]`）
- 字体/图片经 `/api/themes/:slug/assets/*` 托管（防目录穿越）
- 内置 `default` 主题自动补建、不可删除，保证任何情况下站点有样式

**工程**

- 统一 REST API：规范化错误形状、分页、RFC3339 UTC 时间戳
- JWT（HS256，7 天）鉴权，argon2 密码哈希
- 未安装门禁：初始化前除白名单外所有 API 返回 503，前端自动进入安装向导

## 技术栈

| 层 | 技术 |
|---|---|
| 后端 | Rust · [Axum](https://github.com/tokio-rs/axum) 0.8 · tokio · [sqlx](https://github.com/launchbadge/sqlx) 0.8（Any 驱动：SQLite/MySQL）· [rhai](https://rhai.rs/) 1.x（插件沙箱）· pulldown-cmark（Markdown 渲染）· zip · jsonwebtoken · argon2 |
| 前端 | React 19 · TypeScript 5.9 · Vite 8 · Tailwind CSS 4 · shadcn/ui 风格组件（Radix UI）· react-router-dom 7 · react-markdown + rehype-highlight · sonner |

## 快速开始

依赖要求：

- **Rust**：stable 工具链（含 cargo）
- **Node.js**：`^20.19.0 || >=22.12.0`（Vite 8 要求），含 npm

```bash
# 1. 启动后端（默认监听 127.0.0.1:3000）
cd backend
cargo run

# 2. 启动前端 dev server（端口 5173，/api 自动代理到 3000）
cd frontend
npm install
npm run dev
```

浏览器访问 **http://localhost:5173**，未安装时自动进入安装向导：

1. 选择数据库：SQLite（默认，写入 `backend/reedblog.db`，零配置）或 MySQL（填连接信息）；
2. 设置管理员用户名/密码与站点标题、副标题；
3. 完成安装（后端写入 `backend/config.toml`，无需重启），随后即可访问前台与后台
   （`/admin/login` 登录）。

配置文件路径默认为工作目录下的 `config.toml`，可用环境变量 `REEDBLOG_CONFIG` 覆盖；
监听地址、CORS 来源、插件/主题目录等均在 config.toml 中配置。

## 生产部署

```bash
# 构建后端二进制
cd backend
cargo build --release        # → backend/target/release/reedblog-backend(.exe)

# 构建前端静态产物
cd frontend
npm ci
npm run build                # → frontend/dist/
```

部署形态：`dist/` 交给 Nginx 托管（SPA fallback 到 `index.html`），`/api` 反向代理
到后端进程；后端以 `127.0.0.1:3000` 运行在 Nginx 之后（同源反代无 CORS 问题，
`[cors] allowed_origins` 无需修改）。

Nginx 配置示例：

```nginx
server {
    listen 80;
    server_name blog.example.com;

    # 前端静态资源 + SPA fallback
    root /var/www/reedblog/dist;
    index index.html;
    location / {
        try_files $uri /index.html;
    }

    # 后端 API 反向代理
    location /api/ {
        proxy_pass http://127.0.0.1:3000;
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;
    }

    # 插件/主题 zip 上传的后端上限是 32MB，Nginx 需对齐
    client_max_body_size 32m;
}
```

含 SEO 爬虫分流的完整片段见 [`deploy/nginx.conf.example`](deploy/nginx.conf.example)
（**本机未安装 Nginx，该片段未实机验证**，上线前请按文件头部注释自测）。

运行后端（工作目录建议固定，`config.toml`、`plugins/`、`themes/`、SQLite 数据库
文件都相对它解析）：

```bash
cd /opt/reedblog
REEDBLOG_CONFIG=/opt/reedblog/config.toml /usr/local/bin/reedblog-backend
```

首次部署通过 `https://blog.example.com/install` 完成安装向导。默认监听
`127.0.0.1`，适合反代部署；如需直接暴露，改 config.toml `[server] host`。
生产建议再用 systemd 等进程管理器托管后端。

## Docker 部署

不想手工装 Rust/Node/Nginx？仓库自带两个镜像（后端 + Nginx/前端）与 compose 编排：

```bash
docker compose up -d --build
# 打开 http://localhost:8080 → 首次访问走 /install 安装向导
```

- 全部数据在一个命名卷里（`/data`：config.toml、SQLite、uploads、plugins、themes），删容器不丢数据；
- 默认 SQLite 单机，可选 `--profile mysql` 切 MySQL；
- 镜像发到 GHCR：`ghcr.io/langfordkuo/reedblog-backend`、`ghcr.io/langfordkuo/reedblog-web`
  （linux/amd64 + linux/arm64），tag `vX.Y.Z` / `X.Y` / `latest` / `sha-<短sha>`；
- 后端镜像有两条构建路径：本机 `docker build` / compose 是容器内编译的自包含路径（零前置条件，首次较慢）；
  CI 走「runner 先编二进制、镜像只组装」的快路径（`deploy/docker/Dockerfile.runtime`），产物运行时内容一致
  ——见 [deploy/DOCKER.md](deploy/DOCKER.md) 第 10 节。

完整说明（数据卷与备份、切 MySQL、环境变量、常见问题）见
[deploy/DOCKER.md](deploy/DOCKER.md)。

## SEO / 分享元信息（爬虫分流）

SPA 的运行时 meta 注入微信 / Twitter / Facebook / Slack 等抓取器**看不到**（它们不执行 JS），
因此采用方案 a：**Nginx 按 UA 分流**（契约「SEO / 分享元信息」条款）。

- **真人流量**：Nginx 照常返回 SPA 静态文件，行为不变；`frontend/src/lib/meta.ts` 在运行时
  注入 `document.title`、`meta[name=description]`、`og:title/og:description/og:url` 与
  `link[rel=canonical]`（标签统一带 `data-reedblog-meta="1"` 标注，不与插件 head 注入打架）。
- **爬虫 / 社媒预览**（`bot|crawler|spider|facebookexternalhit|…|micromessenger|wechat`，大小写不敏感）：
  `/posts/*`、`/pages/*` 被 proxy 到后端，返回带 OG/Twitter/JSON-LD 的最小 HTML；
  `/robots.txt` 同样走后端（含 `Sitemap: {base}/api/sitemap.xml` 与后台 Disallow）。
- 未命中白名单的请求若直接打到后端 → **302 回站点根**，后端不会把裸 HTML 给真人。
- `og:image` 选取链：文章正文第一张 Markdown 图 → 站点设置 `og_image`（后台「站点管理」新增，
  公开可读）→ 都没有则省略该标签。
- 实现：`backend/src/seo.rs`（渲染/转义/UA 白名单，纯逻辑）+ `backend/src/handlers/seo.rs`（HTTP）。

自测（后端直连）：

```bash
curl -sA "Twitterbot/1.0" http://127.0.0.1:3000/posts/<slug> | head -30   # OG HTML
curl -sA "Mozilla/5.0" -D - -o /dev/null http://127.0.0.1:3000/posts/<slug>  # 期望 302
curl -s http://127.0.0.1:3000/robots.txt                                   # 期望含 Sitemap:
```

完整 Nginx 分流片段见 [`deploy/nginx.conf.example`](deploy/nginx.conf.example)
（**未在真实 Nginx 上实机验证**）。

## 目录结构

```
reedblog/
├── backend/                  # Rust 后端（Axum）
│   ├── src/
│   │   ├── handlers/         # API 处理器（安装/公开/管理/插件/主题）
│   │   ├── plugins.rs        # 插件宿主：Rhai 沙箱 + 钩子链 + 注入片段
│   │   ├── themes.rs         # 主题系统：manifest、内置 default、静态托管
│   │   ├── packages.rs       # zip 解压校验、slug/semver 工具
│   │   ├── config.rs         # config.toml 结构与读写
│   │   ├── state.rs          # 应用状态、数据库连接与迁移
│   │   ├── auth.rs / error.rs / middleware.rs / models.rs
│   │   └── main.rs / lib.rs  # 入口与路由组装
│   ├── migrations/           # sqlx 迁移（sqlite/ 与 mysql/ 各一份）
│   └── tests/                # 集成测试（integration、extensibility、uploads_feed、media 等）
├── frontend/                 # React 前端（Vite + Tailwind CSS 4）
│   └── src/
│       ├── pages/            # 公开页面、/install 安装向导、/admin 后台
│       ├── components/       # shadcn/ui 组件与站点组件（含插件注入）
│       └── lib/              # API 客户端、主题应用、类型定义
├── docs/                     # API 契约与开发文档
└── examples/                 # 示例插件 / 示例主题源码
```

## 文档

| 文档 | 说明 |
|---|---|
| [docs/api-contract.md](docs/api-contract.md) | 核心 API 契约 v1（端点、数据形状、错误码） |
| [docs/extensibility-contract.md](docs/extensibility-contract.md) | 扩展系统契约 v1（插件 + 主题的前后端共同规格） |
| [docs/plugin-development.md](docs/plugin-development.md) | 插件开发指南（manifest、Rhai 钩子、沙箱、注入、打包） |
| [docs/theme-development.md](docs/theme-development.md) | 主题开发指南（令牌清单、theme.css、assets、打包） |
| [examples/plugins/demo-suite/](examples/plugins/demo-suite/) | 示例插件：评论违禁词过滤 + 版权页脚 + 统计注入 |
| [examples/themes/midnight/](examples/themes/midnight/) | 示例主题：覆盖全部令牌的暗色主题 |

## 开发

```bash
# 后端：格式检查 + 单元测试 + 集成测试（API 全链路与扩展系统验收）
cd backend
cargo fmt --check
cargo test --all-targets
# 生成示例插件/主题 zip 到 tests/fixtures/（可选）
cargo test --test extensibility generate_fixtures -- --ignored

# 前端：类型检查、生产构建与单元测试
cd frontend
npm ci
npm run typecheck     # tsc --noEmit
npm run build         # tsc --noEmit && vite build
npm test              # vitest run（src/lib 下的纯函数与 DOM 单测）
npm run test:watch    # vitest 监听模式（本地开发用）
```

### CI 与测试

推送或提 PR 到 `main` 时，GitHub Actions（[.github/workflows/ci.yml](.github/workflows/ci.yml)）
并行运行两个 job：

- **Backend (Rust)**：`cargo fmt --check` 格式门禁 + `cargo test --all-targets`；
  测试用临时 SQLite 数据库，不依赖任何外部服务或密钥。
- **Frontend (Node 24)**：`npm ci` → `npx tsc --noEmit` → `npm run build` → `npm test`
  （vitest + jsdom，用例位于 `frontend/src/lib/*.test.ts`，覆盖 diff / 数学公式预处理 /
  TOC 提取 / 格式化工具 / API 限流错误解析）。

CI 无需配置密钥；触发路径限定为 `backend/**`、`frontend/**`、`.github/workflows/**`，
同一分支的重复触发会自动取消上一次运行。

端口约定：后端 3000（config.toml `[server]` 可改）；前端 dev server 5173，
`/api` 代理到后端。

## License

[GPL-3.0](LICENSE)
