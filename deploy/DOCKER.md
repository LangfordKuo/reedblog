# Docker 部署 reedblog

两个镜像 + `docker-compose.yml` 编排：

| 镜像 | 内容 | 镜像内端口 |
|---|---|---|
| `ghcr.io/langfordkuo/reedblog-backend` | 后端进程（Rust/Axum），非 root（uid 10002）运行 | 3000 |
| `ghcr.io/langfordkuo/reedblog-web` | Nginx + 前端构建产物（含 `/api` 反代与 UA 爬虫分流） | 80 |

数据全部落在 `backend` 容器的 `/data` 卷：`config.toml`、SQLite 文件、`uploads/`、`plugins/`、`themes/`。

---

## 1. 快速开始（SQLite 单机，默认）

```bash
# 仓库根目录
docker compose up -d --build
```

浏览器打开 **http://localhost:8080** → 首次访问进入 `/install` 安装向导（填管理员账号、站点信息；
数据库选 SQLite 即可）→ 完成后站点即可用。

- 默认端口 **8080**（web/Nginx）。后端只在 compose 网络内以 `backend:3000` 提供，不对宿主机暴露。
- 装完后想再确认状态：`curl -i http://localhost:8080/api/site/settings` 应为 `200`；
  未安装时该接口是 `503 not_installed`（镜像健康检查把 503 也视为健康，安装前不会一直 unhealthy）。

停止 / 更新：

```bash
docker compose down            # 停止并删除容器（数据卷保留）
docker compose up -d --build   # 重新构建并启动
```

## 2. 数据卷与备份

| 卷 | 内容 |
|---|---|
| `reedblog-data` | `/data`：`config.toml`、SQLite 文件（默认 `/data/reedblog.db`）、`uploads/`（图片）、`plugins/`、`themes/` |
| `reedblog-mysql` | 仅 MySQL 模式：MySQL 数据目录 |

**备份**（在线冷备，站点数据全在卷里）：

```bash
docker run --rm -v reedblog-data:/data -v "$PWD":/backup alpine \
  tar czf /backup/reedblog-data-$(date +%F).tgz -C /data .
```

**恢复**：

```bash
docker compose down
docker run --rm -v reedblog-data:/data -v "$PWD":/backup alpine \
  sh -c 'rm -rf /data/* && tar xzf /backup/reedblog-data-2026-10-04.tgz -C /data'
docker compose up -d
```

> 也可以用后台「备份」页导出 zip（数据 + 媒体），但注意 Nginx 默认 `client_max_body_size 32m`，
> 大于 32MB 的 zip 恢复会被代理层拦下——见「常见问题」。

## 3. 切换到 MySQL

1. 复制 `.env.example` 为 `.env`，改密码，并把 `REEDBLOG_DB_TYPE` 设为 `mysql`；
2. 启动：

```bash
docker compose --profile mysql up -d --build
```

3. 浏览器打开 http://localhost:8080/install，数据库选 **MySQL**，host 填 **`mysql`**（compose 服务名）、
   端口 `3306`，用户名/密码/库名与 `.env` 一致。

说明：
- `mysql` 服务带 `profiles: [mysql]`，不启用 profile 时完全不会被创建；`backend` 对它的
  `depends_on` 标了 `required: false`（需要 Docker Compose v2.20+），默认模式下自动忽略。
- yml 里 backend 的 `REEDBLOG_MYSQL_*` 与 mysql 服务共用同一组 `.env` 变量（`MYSQL_HOST`/`MYSQL_PORT`/
  `MYSQL_USER`/`MYSQL_PASSWORD`/`MYSQL_DATABASE`），只需在一处维护。
- **容器入口只在生成 `config.toml` 时读这些环境变量**：若数据卷里已经有 `config.toml`（例如已经用
  SQLite 装过），改 `.env` 不会换库。要么在安装向导里重装到 MySQL，要么手工改 `/data/config.toml`
  后重启：把 `[database] db_type` 改成 `mysql` 并补上 `[database.mysql]` 段（`host = "mysql"` 等）。

## 4. 直接拉 GHCR 镜像跑（不用 compose）

```bash
# 后端（数据卷、端口 3000）
docker run -d --name reedblog-backend \
  -p 3000:3000 -v reedblog-data:/data \
  --restart unless-stopped \
  ghcr.io/langfordkuo/reedblog-backend:latest

# Web（同机时用 host.docker.internal 指向宿主机后端；Linux 可加 --add-host host.docker.internal:host-gateway）
docker run -d --name reedblog-web \
  -p 8080:80 -e BACKEND_UPSTREAM=host.docker.internal:3000 \
  --restart unless-stopped \
  ghcr.io/langfordkuo/reedblog-web:latest
```

> `BACKEND_UPSTREAM` 是 `host:port`，**不带 `http://` 前缀**（模板里拼成 `http://$BACKEND_UPSTREAM`）。
> 只跑后端镜像时，浏览器拿到的是 API 响应，没有前端页面——正常，前端在 web 镜像里。

## 5. 升级

```bash
docker compose pull          # 拉取新版本镜像（默认 latest；生产建议固定 :X.Y）
docker compose up -d         # 重建容器，数据卷不动
```

- 数据库迁移在后端启动时自动执行，升级无需手工操作。
- 想固定版本：把 `docker-compose.yml` 里的 `image:` 改成 `ghcr.io/langfordkuo/reedblog-backend:X.Y`。
- 用本地源码构建则用 `docker compose up -d --build`（会先 git pull）。

## 6. 环境变量（backend 容器）

入口脚本 `deploy/docker/entrypoint.sh` 在 `/data/config.toml` **不存在**时按这些变量生成未安装态配置；
已存在则原样使用，变量只在日志里提示。

| 变量 | 默认值 | 说明 |
|---|---|---|
| `REEDBLOG_CONFIG` | `/data/config.toml` | 配置文件路径 |
| `REEDBLOG_PORT` | `3000` | 监听端口（`[server] host` 固定 `0.0.0.0`） |
| `REEDBLOG_DB_TYPE` | `sqlite` | `sqlite` 或 `mysql` |
| `REEDBLOG_SQLITE_PATH` | `/data/reedblog.db` | SQLite 文件路径 |
| `REEDBLOG_MYSQL_HOST` | `mysql` | MySQL 主机 |
| `REEDBLOG_MYSQL_PORT` | `3306` | MySQL 端口 |
| `REEDBLOG_MYSQL_USER` | `reedblog` | MySQL 用户名 |
| `REEDBLOG_MYSQL_PASSWORD` | 空 | MySQL 密码 |
| `REEDBLOG_MYSQL_DATABASE` | `reedblog` | MySQL 库名 |
| `REEDBLOG_CORS_ORIGINS` | 空 | 跨域白名单，逗号分隔（同源反代不需要） |

web 容器：`BACKEND_UPSTREAM`（默认 `backend:3000`）。SMTP 密码沿用后端既有约定
`REEDBLOG_SMTP_PASSWORD`，需要时自行 `docker compose` 追加环境变量即可。

## 7. 镜像与 tag 规则

- 双架构：`linux/amd64`、`linux/arm64`（同一 tag 的多平台 manifest）。
- tag push（`v1.2.3`）→ `v1.2.3`、`1.2`、`latest`、`sha-<短sha>` 四个 tag；
- push main / PR → 只构建校验，不推送；
- CI 里还有一个单架构冒烟测试 job：真起容器验证「未安装态 503 → 安装 201 → web 首页 200 → `/api` 反代可达
  → 爬虫 UA 分流到后端」。

## 8. 常见问题

**端口 8080 被占用** — 改 `docker-compose.yml` 的 `web.ports` 左侧，如 `"18080:80"`。

**卷属主 / 权限（`数据目录不可写`）** — 容器以 uid 10002(`reedblog`) 运行；命名卷首次挂载会继承镜像内
`/data` 的属主，正常无需处理。若用**绑定挂载**宿主机目录（`-v /srv/reedblog:/data`），要先：

```bash
sudo chown -R 10002:10002 /srv/reedblog
```

**上传/恢复大小** — 图片上传上限由 `[uploads] max_size_mb`（默认 10MB）控制；插件/主题 zip 与 Nginx 的
`client_max_body_size` 都是 32MB。后台「备份导入」后端上限是 1GiB，如需恢复大于 32MB 的备份 zip，把
`deploy/docker/nginx.conf.template` 里 `client_max_body_size 32m` 改成 `1024m` 后重建 web 镜像。

**爬虫/社媒分享没有 OG 卡片** — 分流在 **web 镜像的 Nginx** 里（UA 白名单与后端
`backend/src/seo.rs::is_crawler_ua` 一致），必须同时跑 web + backend 两个容器；只跑后端镜像不会按 UA 分流。
自测：

```bash
curl -sA "Twitterbot/1.0" http://localhost:8080/posts/<slug> | head -20   # OG HTML
curl -s http://localhost:8080/robots.txt                                  # 含 Sitemap:
```

**改了 `config.toml` 不生效** — 改的是卷里的 `/data/config.toml` 吗？改完要 `docker compose restart backend`。
入口脚本绝不覆盖已存在的配置。

**健康检查一直 unhealthy** — 镜像内探测 `GET /api/site/settings`（200 或 503 都算健康）；若持续失败，
`docker compose logs backend` 看后端是否起来（端口被改、配置解析失败等）。首次启动 web 会等后端 healthy，
大约多等 10 秒左右。

**只重建了 backend，web 报 502** — Nginx 在启动时解析一次 `backend` 的地址；若单独 `--force-recreate backend`
导致 IP 变化，`docker compose restart web` 让 Nginx 重新解析即可（`docker compose up -d` 会一并重建，无此问题）。

**用 32 位/其他架构设备** — 只发布 amd64/arm64，其他平台需自行构建。

## 9. 说明与边界

- 本仓库的开发机没有 Docker，`Dockerfile` / `docker-compose.yml` / CI workflow 是**静态校验 + 首次 CI
  运行**验证的：CI 的 `smoke` job 会在 GitHub runner 上真跑一遍两个镜像（见 `.github/workflows/docker.yml`）。
- 后端进程目前未实现优雅停机处理，容器 `SIGTERM` 走默认终止；`exec` 已保证信号直达后端进程（不是被 sh 吞掉）。
- 备份请以「数据卷打包」为准（见第 2 节），它涵盖 `config.toml`（含 `jwt_secret`）与全部媒体文件。
