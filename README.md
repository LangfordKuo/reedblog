# reedblog

[English](README.en.md) | **中文**

使用 Rust 编写的轻量化博客系统，带插件与主题扩展（Rust/Axum 后端 + React 前端，SQLite 或 MySQL）。

## 快速开始（Docker）

镜像发在 GHCR（`linux/amd64` + `linux/arm64`），**一条命令跑起整套**（Nginx 与后端在同一容器）：

```bash
docker run -d --name reedblog -p 8080:80 -v reedblog-data:/data \
  --restart unless-stopped ghcr.io/langfordkuo/reedblog:latest
```

打开 **http://localhost:8080** → 首次访问自动进入 `/install` 安装向导（数据库选 SQLite 即可，零配置）。
数据全在 `reedblog-data` 卷里（`config.toml`、SQLite、`uploads/`、`plugins/`、`themes/`），删容器不丢；
升级时 `docker pull` 后删旧容器、用同一条命令带同一卷重跑。

**国内服务器拉不动 `ghcr.io`？** 把镜像前缀 `ghcr.io/` 换成加速地址 `ghcr.1ms.run/` 即可（第三方公共服务，非本项目提供，可用性与凭据安全请自行确认；实测三个镜像均可经它匿名拉取）：

```bash
docker pull ghcr.1ms.run/langfordkuo/reedblog:latest
docker tag  ghcr.1ms.run/langfordkuo/reedblog:latest ghcr.io/langfordkuo/reedblog:latest  # 需要保留原镜像名时（例如给 compose 用）
```

> 该镜像内 Nginx 以 root 运行（需要绑定 80 端口）。要进程隔离与后端非 root（uid 10002）请用下面的方案。

其他部署方式：

| 方式 | 做法 |
|---|---|
| 两个镜像分开跑 | `reedblog-backend` + `reedblog-web`，需先建网络并把 `BACKEND_UPSTREAM` 指向后端容器，见 [DOCKER.md](deploy/DOCKER.md) |
| docker compose（推荐长期使用） | `docker compose pull && docker compose up -d`，可选 `--profile mysql` 切 MySQL |
| 源码运行 | 见[源码运行](#源码运行本地开发) |
| 不用 Docker 上线 | 见[生产部署](#生产部署自建-nginx) |

镜像 tag 为 `vX.Y.Z` / `X.Y` / `latest` / `sha-<短sha>`。完整说明（三个镜像对比、数据卷与备份、切 MySQL、环境变量、升级、常见问题）见 [deploy/DOCKER.md](deploy/DOCKER.md)。

## 功能

**内容**：Markdown 写作（GFM 表格/删除线/任务列表）、草稿与发布、自定义或自动 slug、摘要缺省自动提取；分类、标签、按月归档、分页；全文搜索（LIKE 实现，SQLite/MySQL 零迁移）

**评论**：楼中楼回复；**审核方式可切换**（先发后审 / 先审后发）；反滥用（同 IP+目标限流、蜜罐、关键词黑名单、登录失败退避）；SMTP 邮件通知（密码只从配置或环境变量读取，绝不入库、不经接口返回）

**媒体**：编辑器工具栏/粘贴/拖拽上传（magic bytes 判真实类型、sha256 去重、SVG 拒绝）；后台媒体库（网格浏览、复制 URL、删除）

**展示**：代码高亮、KaTeX 公式、暗色切换（light/dark/system，首绘前应用防闪烁）、相关文章、上一篇/下一篇、阅读进度、点赞与浏览量、RSS、sitemap、SEO 爬虫分流（爬虫与社媒预览拿到 OG/Twitter/JSON-LD）

**后台**：仪表盘、文章与页面、修订历史、回收站、评论、分类标签、媒体库、主题、插件、站点设置、备份导出与导入

**扩展**：插件 = 目录 + [Rhai](https://rhai.rs/) 脚本 + manifest，zip 上传即装（4 个后端钩子 + 前端 `head`/`body_end` 注入，沙箱无文件/网络/进程能力）；主题 = 设计令牌 + CSS + 资源，zip 上传一键激活（32 个令牌覆盖 shadcn/Tailwind 变量，内置 `default` 主题不可删）。详见[插件开发](docs/plugin-development.md)、[主题开发](docs/theme-development.md)

**工程**：统一 REST API（规范化错误形状、分页、RFC3339 UTC）；JWT（HS256，7 天）+ argon2；安装向导；未安装门禁（初始化前除白名单外一律 503）

## 技术栈

| 层 | 技术 |
|---|---|
| 后端 | Rust · [Axum](https://github.com/tokio-rs/axum) 0.8 · tokio · [sqlx](https://github.com/launchbadge/sqlx) 0.8（Any 驱动：SQLite/MySQL）· pulldown-cmark · [Rhai](https://rhai.rs/)（插件沙箱）· lettre（SMTP） |
| 前端 | React 19 · TypeScript · Vite 8 · Tailwind CSS 4 · shadcn/ui 风格组件（Radix UI）· react-router 7 · KaTeX · highlight.js |

## 源码运行（本地开发）

依赖：Rust stable（含 cargo）、Node.js `^20.19.0 || >=22.12.0`。

```bash
cd backend  && cargo run                      # 后端 127.0.0.1:3000
cd frontend && npm install && npm run dev     # 前端 5173，/api 自动代理到 3000
```

打开 **http://localhost:5173** → 未安装时自动进入安装向导：选数据库（SQLite 零配置，或 MySQL）→ 设管理员与站点信息 → 完成后写入 `backend/config.toml`（无需重启），后台入口 `/admin/login`。

## 生产部署（自建 Nginx）

```bash
cd backend  && cargo build --release     # → backend/target/release/reedblog-backend
cd frontend && npm ci && npm run build   # → frontend/dist/
```

`dist/` 交给 Nginx 托管（SPA fallback 到 `index.html`），`/api` 反代到后端；后端保持监听 `127.0.0.1:3000`（同源反代无需改 `[cors]`）。完整片段（含 SEO 爬虫分流）见 [`deploy/nginx.conf.example`](deploy/nginx.conf.example)，可执行包的三步部署见 [`deploy/QUICKSTART.md`](deploy/QUICKSTART.md)。建议用 systemd 托管后端并固定工作目录（`config.toml`、`plugins/`、`themes/`、SQLite 都相对它解析），首次部署走 `/install`。

配置已标记安装（`[auth] jwt_secret` 非空）但数据库连接或迁移失败时，后端会**拒绝启动（非零退出码）**并打印醒目错误日志，而不是进入安装向导——避免管理员误走 `/install` 覆盖已有数据；升级前请先备份数据库与 `uploads/` 上传目录。

## 配置

`config.toml` 默认位于运行目录，可用环境变量 `REEDBLOG_CONFIG` 指定路径（安装向导会把配置写回该文件）。

| 项 | 说明 |
|---|---|
| `[server] host` / `port` / `base_url` | 监听地址与端口；`base_url` 为空时按反代头（X-Forwarded-Proto/Host）推导绝对 URL |
| `[database] db_type` / `sqlite_path` / `[database.mysql]` | SQLite 文件路径，或 MySQL 连接信息 |
| `[auth] jwt_secret` | JWT 密钥（安装时自动生成）；为空即未安装态 |
| `[cors] allowed_origins` | 前后端分离调试时需要；同源反代无需改动 |
| `[plugins]` / `[themes]` / `[uploads]` | 插件、主题、上传目录；`[uploads] max_size_mb` 为单文件上限 |
| `[smtp] password` | 只从此处或环境变量 `REEDBLOG_SMTP_PASSWORD`（优先）读取，绝不入库 |

## 目录结构

```
backend/   Rust 后端：handlers 处理器、plugins.rs 插件宿主、themes.rs 主题系统、seo.rs 爬虫分流、
           migrations 双方言迁移、tests 集成测试
frontend/  React 前端：src/pages 公开页面 + /install + /admin、components、lib（API 客户端、主题应用）
docs/      API 契约与开发文档        deploy/  部署片段与 Docker 说明        examples/  示例插件与主题
```

## 文档

| 文档 | 说明 |
|---|---|
| [docs/api-contract.md](docs/api-contract.md) | 核心 API 契约 v1（端点、数据形状、错误码） |
| [docs/extensibility-contract.md](docs/extensibility-contract.md) | 扩展系统契约 v1（插件 + 主题共同规格） |
| [docs/plugin-development.md](docs/plugin-development.md) | 插件开发指南 |
| [docs/theme-development.md](docs/theme-development.md) | 主题开发指南 |
| [deploy/DOCKER.md](deploy/DOCKER.md) | Docker 部署完整说明 |
| [deploy/QUICKSTART.md](deploy/QUICKSTART.md) | 可执行包的三步部署 |
| [examples/](examples/) | 示例插件 demo-suite、示例主题 midnight |

## 开发与 CI

```bash
cd backend  && cargo fmt --check && cargo test --all-targets
cd frontend && npm ci && npm run typecheck && npm run build && npm test
```

推送到 `main` 或提 PR 时，GitHub Actions 并行跑后端（格式门禁 + 全量测试）与前端（类型检查 + 构建 + vitest 单测），配置见 [ci.yml](.github/workflows/ci.yml)。打 tag 时另有两条流水线：[release.yml](.github/workflows/release.yml) 产出三平台可执行包，[docker.yml](.github/workflows/docker.yml) 构建并推送三个镜像（amd64 + arm64）。CI 全程无需密钥。

## License

[GPL-3.0](LICENSE)
