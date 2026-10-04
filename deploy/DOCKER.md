# Docker 部署 reedblog

三个镜像 + `docker-compose.yml` 编排：

| 镜像 | 内容 | 镜像内端口 |
|---|---|---|
| `ghcr.io/langfordkuo/reedblog` | **一体化**：Nginx（前端 + `/api` 反代 + UA 爬虫分流）+ 后端进程，同容器运行（容器内 root） | 80 |
| `ghcr.io/langfordkuo/reedblog-backend` | 后端进程（Rust/Axum），非 root（uid 10002）运行 | 3000 |
| `ghcr.io/langfordkuo/reedblog-web` | Nginx + 前端构建产物（含 `/api` 反代与 UA 爬虫分流） | 80 |

数据全部落在 `/data` 卷：`config.toml`、SQLite 文件、`uploads/`、`plugins/`、`themes/`。
一体化镜像与 backend 镜像共用同一份入口逻辑（`deploy/docker/entrypoint-lib.sh`：数据目录检查 +
未安装态配置生成），行为一致。

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

### 4.1 一体化镜像：一条命令跑整套

Nginx（前端 + `/api` 反代 + UA 分流）与后端在同一个容器里，最省事：

```bash
docker run -d --name reedblog -p 8080:80 -v reedblog-data:/data \
  --restart unless-stopped ghcr.io/langfordkuo/reedblog:latest
```

打开 **http://localhost:8080** → 首次访问进入 `/install` 安装向导。升级：

```bash
docker pull ghcr.io/langfordkuo/reedblog:latest
docker rm -f reedblog && docker run -d --name reedblog -p 8080:80 -v reedblog-data:/data \
  --restart unless-stopped ghcr.io/langfordkuo/reedblog:latest     # 数据卷不变
```

**与 compose 方案（两个镜像）的取舍**：

| | 一体化 `reedblog` | compose 的 backend + web |
|---|---|---|
| 启动 | 一条 `docker run` | `docker compose up -d` |
| 运行身份 | nginx 与后端同容器，**容器内 root**（nginx 要绑 80、写 `/var/cache/nginx` 与 `/var/run`） | 两个容器各自隔离，后端非 root（uid 10002） |
| 前后端独立升级 / 扩缩副本 | 不行，整体升级 | 可以，还能换成外部 MySQL / 独立反代 |
| 数据卷 | `-v <卷>:/data` | 同一个卷挂到 backend 容器 |
| 端口 | 宿主机 → 容器 `80` | 宿主机 → web 容器 `80`（backend 只在 compose 网络内） |
| 适合 | 先跑起来、单机小站、演示 | 长期生产、要隔离、要扩展 |

一体化镜像的运行约定与 backend 镜像一致：`/data` 卷、`REEDBLOG_CONFIG=/data/config.toml`、
未安装态 503 也算健康、环境变量（`REEDBLOG_PORT`/`REEDBLOG_DB_TYPE`/…）只在首次生成
`config.toml` 时生效、绝不覆盖已有配置。差异只有两点：

- **root 运行**（原因见上表）。也因此它写入 `/data` 的文件属主是 root；若之后换用 compose 的
  非 root 后端，先修正属主：绑定挂载用 `sudo chown -R 10002:10002 <宿主机目录>`，
  命名卷可临时起容器执行同样命令。
- `BACKEND_UPSTREAM` 在镜像里默认 **`127.0.0.1:3000`**（nginx 与后端同机）；改 `REEDBLOG_PORT`
  时必须同步改它，否则 nginx 反代不到后端（入口检测到两者不一致会在日志里打印警告）。

### 4.2 分开跑：后端 + Web 两个镜像

```bash
# 网络 + 数据卷
docker network create reedblog
docker volume create reedblog-data

# 后端（数据卷、仅在容器网络内提供服务）
docker run -d --name reedblog-backend --network reedblog --network-alias backend \
  -v reedblog-data:/data --restart unless-stopped \
  ghcr.io/langfordkuo/reedblog-backend:latest

# Web（BACKEND_UPSTREAM 指向后端容器名）
docker run -d --name reedblog-web --network reedblog -p 8080:80 \
  -e BACKEND_UPSTREAM=backend:3000 --restart unless-stopped \
  ghcr.io/langfordkuo/reedblog-web:latest
```

打开 http://localhost:8080 → 首次访问进入 `/install` 安装向导。
不用自定义网络时，也可以让后端发布到宿主机、web 用
`-e BACKEND_UPSTREAM=host.docker.internal:3000` 指过来（Linux 需加 `--add-host host.docker.internal:host-gateway`）。

> `BACKEND_UPSTREAM` 是 `host:port`，**不带 `http://` 前缀**（模板里拼成 `http://$BACKEND_UPSTREAM`）。
> 只跑后端镜像时，浏览器拿到的是 API 响应，没有前端页面——正常，前端在 web 镜像里。

## 5. 升级

```bash
docker compose pull          # 拉取新版本镜像（默认 latest；生产建议固定 :X.Y）
docker compose up -d         # 重建容器，数据卷不动
```

- 数据库迁移在后端启动时自动执行，升级无需手工操作。
- **迁移失败即拒绝启动**：config 已标记安装但数据库连接/迁移失败（如已应用的迁移文件被改动、库与二进制版本不匹配）时，后端以非零退出码退出并打印醒目错误日志，**不会**进入安装向导（避免误安装覆盖数据）。容器加了 `--restart` 时表现为反复重启，请用 `docker logs` / `docker compose logs` 看日志排错；**升级前请先备份数据库与上传目录**。
- 想固定版本：把 `docker-compose.yml` 里的 `image:` 改成 `ghcr.io/langfordkuo/reedblog-backend:X.Y`。
- 用本地源码构建则用 `docker compose up -d --build`（会先 git pull）。
- 一体化镜像（第 4.1 节）升级：`docker pull ghcr.io/langfordkuo/reedblog:latest` 后删旧容器、
  用同一条 `docker run` 带同一数据卷重跑；升级前后端一起换，不能只换其中一个。

## 6. 环境变量（backend / 一体化容器）

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

一体化镜像（第 4.1 节）读同一组 `REEDBLOG_*` 变量——入口逻辑与 backend 镜像共用
`deploy/docker/entrypoint-lib.sh`，语义完全一致；额外有一个 `BACKEND_UPSTREAM`
（Nginx 反代目标，镜像默认 `127.0.0.1:3000`，即同容器后端）。

web 容器：`BACKEND_UPSTREAM`（默认 `backend:3000`）。SMTP 密码沿用后端既有约定
`REEDBLOG_SMTP_PASSWORD`，需要时自行 `docker compose` 追加环境变量即可。

## 7. 镜像与 tag 规则

- 三个镜像都发双架构：`linux/amd64`、`linux/arm64`（同一 tag 的多平台 manifest）。
- tag push（`v1.2.3`）→ `v1.2.3`、`1.2`、`latest`、`sha-<短sha>` 四个 tag；
- push main / PR → 只构建校验，不推送；
- 后端镜像与一体化镜像在 CI 里走「runner 先编二进制、镜像只组装」的快路径（见第 10 节），
  不在 QEMU 里跑 cargo（一体化镜像的前端仍在镜像内构建）；
- CI 里还有一个单架构冒烟测试 job，真起容器验证：「后端未安装态 503 → 安装 201」
  「web 首页 200 → `/api` 反代可达 → 爬虫 UA 分流到后端」「一体化镜像静态 200 → `/api` 未安装态 503
  → 安装 201 → 已安装 200」。

## 8. 常见问题

**端口 8080 被占用** — 改 `docker-compose.yml` 的 `web.ports` 左侧，如 `"18080:80"`；
一体化镜像改 `docker run` 的 `-p` 左侧即可。

**一体化镜像为什么以 root 运行？** — nginx 要绑 80 端口、写 `/var/cache/nginx` 与 `/var/run`；
非 root 需要额外处理端口能力与目录属主，与「一条命令先跑起来」的定位不符（文件顶部与第 4.1 节
都写明了这一取舍）。要进程隔离/最小权限请用 compose 的两个镜像（后端非 root）。另外它写入 `/data`
的文件属主是 root，之后改用 compose 前后端方案前要先修属主（见第 4.1 节）。

**一体化镜像页面 502 或容器 unhealthy** — nginx 的反代目标是 `BACKEND_UPSTREAM`（镜像内默认
`127.0.0.1:3000`）。若改过 `REEDBLOG_PORT` 却没同步改它，就会 502；`docker logs <容器名>` 里
nginx 与后端的日志都有，先看后端是否起来。

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
  运行**验证的：CI 的 `smoke` job 会在 GitHub runner 上真跑一遍三个镜像（见 `.github/workflows/docker.yml`）。
  后端镜像自引入 CI 快路径后（第 10 节），组装产物同样由 `smoke` job 真跑验证；一体化镜像从加入起就在
  `smoke` 里跑「静态 200 → `/api` 未安装态 503 → 安装 201 → 已安装 200」。
- 后端进程目前未实现优雅停机处理，容器 `SIGTERM` 走默认终止；`exec` 已保证信号直达后端进程（不是被 sh 吞掉）。
  一体化镜像同理：后端是 PID 1，容器停止时 PID 1 退出，内核会清掉同 PID 命名空间里的 nginx，无需额外信号处理。
- 一体化镜像**有意以 root 运行**（原因见第 4.1 节与 `deploy/docker/Dockerfile.allinone` 顶部注释），
  这是它与 backend 镜像（非 root uid 10002）的既定差异，不是配置疏漏。
- 备份请以「数据卷打包」为准（见第 2 节），它涵盖 `config.toml`（含 `jwt_secret`）与全部媒体文件。

## 10. 后端镜像的两条构建路径：本机自包含 vs CI 快路径

两条路径产出的运行时镜像内容一致，按场景选：

| | 本机自包含 | CI 快路径 |
|---|---|---|
| Dockerfile | 仓库根 `Dockerfile` | `deploy/docker/Dockerfile.runtime` |
| 前置条件 | 只要 Docker | 构建前备好 `prebuilt/amd64/`、`prebuilt/arm64/` 二进制 |
| 编译在哪 | 容器内（`cargo build --release --locked`，bundled SQLite 的 C 也一起编） | runner 上：amd64 原生 + arm64 交叉（同 `release.yml` 的 `gcc-aarch64-linux-gnu` 方案） |
| 用途 | 本地开发、离线复现镜像 | `.github/workflows/docker.yml`（build + smoke） |

**本机自包含**（首次全量编译，较慢；compose 走的也是这条）：

```bash
docker build -t reedblog-backend .
docker compose up -d --build        # 等价
```

**CI / 手工组装快路径**（构建上下文 = 仓库根）：

```bash
# 1) 先备好两个架构的二进制（CI 里由 docker.yml 的 binaries job 完成，Swatinem/rust-cache 缓存 target/）
cd backend
cargo build --release --locked --target x86_64-unknown-linux-gnu
# arm64 交叉（Linux；与 release.yml 用同一组变量）
sudo apt-get install -y gcc-aarch64-linux-gnu
CC_aarch64_unknown_linux_gnu=aarch64-linux-gnu-gcc \
AR_aarch64_unknown_linux_gnu=aarch64-linux-gnu-ar \
CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=aarch64-linux-gnu-gcc \
  cargo build --release --locked --target aarch64-unknown-linux-gnu
cd ..

# 2) 摆成 prebuilt/<TARGETARCH>/ 布局后组装镜像（Dockerfile.runtime 用 buildx 的 TARGETARCH 选二进制）
mkdir -p prebuilt/amd64 prebuilt/arm64
cp backend/target/x86_64-unknown-linux-gnu/release/reedblog-backend prebuilt/amd64/
cp backend/target/aarch64-unknown-linux-gnu/release/reedblog-backend prebuilt/arm64/
docker buildx build --platform linux/amd64,linux/arm64 \
  -f deploy/docker/Dockerfile.runtime \
  -t ghcr.io/langfordkuo/reedblog-backend:dev --push .
```

> 为什么要这条快路径：原先后端镜像直接在 buildx 里构建，arm64 要在 QEMU 里全量 `cargo build`
> （含 bundled SQLite 的 C 编译），单次 30 分钟以上；且 BuildKit 的 cache mount 不进 gha 缓存，
> 等于每次全量重编。改成 runner 预编译 + rust-cache 缓存 `backend/target` 后，热缓存只有增量编译。
> 两份 Dockerfile 的 runtime 阶段（`debian:bookworm-slim`、`ca-certificates`+`curl`、非 root uid 10002、
> `/data` 预建属主、`REEDBLOG_CONFIG`、`VOLUME`、`EXPOSE 3000`、HEALTHCHECK 200/503、entrypoint）
> 逐项一致，只有二进制来源不同——改动运行时行为请两份一起改。

**一体化镜像（`deploy/docker/Dockerfile.allinone`）** 只有 CI 快路径：它的后端二进制同样来自
`binaries` job 的 `prebuilt/<TARGETARCH>/`，前端在镜像内用 `node:24-alpine` 构建，基础镜像是
`nginx:stable-bookworm`（Debian/glibc；`nginx:alpine` 是 musl，跑不了 gnu 产物），
入口与 backend 镜像共用 `deploy/docker/entrypoint-lib.sh`。手工构建示例：

```bash
# 先备好 prebuilt/amd64/、prebuilt/arm64/（步骤同上）
docker buildx build --platform linux/amd64,linux/arm64 \
  -f deploy/docker/Dockerfile.allinone \
  -t ghcr.io/langfordkuo/reedblog:dev --push .
```

## 11. 国内网络：ghcr.io 镜像加速

国内机器拉 `ghcr.io` 可能超时。可用第三方加速地址 **`ghcr.1ms.run`**：把镜像前缀 `ghcr.io/` 换成
`ghcr.1ms.run/` 即可（**该服务非本项目提供**，可用性与凭据安全请自行判断，不建议生产环境长期依赖单一加速站）。

**a) 直接拉（docker run）**

```bash
docker pull ghcr.1ms.run/langfordkuo/reedblog:latest
docker run -d --name reedblog -p 8080:80 -v reedblog-data:/data \
  --restart unless-stopped ghcr.1ms.run/langfordkuo/reedblog:latest
```

需保留原镜像名时（例如后续要给 compose / 已有脚本用）加一步改名：

```bash
docker tag ghcr.1ms.run/langfordkuo/reedblog:latest ghcr.io/langfordkuo/reedblog:latest
```

**b) compose**：`docker-compose.yml` 里的 `image:` 是 `ghcr.io/...`，两种做法——

- 按上面 `docker pull` + `docker tag` 把原名镜像拉到本地，再 `docker compose up -d`（不再联网拉）；
- 或临时把 compose 里三处 `image: ghcr.io/` 改成 `image: ghcr.1ms.run/`（仅本机生效，别提交回仓库）。

**c) 三个镜像都适用**：`reedblog`（一体化）、`reedblog-backend`、`reedblog-web`，tag 规则与 ghcr.io 一致。

**已验证**：在匿名前提下（不带任何凭据）从加速站拉取三个镜像的 `latest` 与 `v0.3.0` manifest 均返回 200，
且返回的是**同一个多架构清单**（`linux/amd64` + `linux/arm64`），与直连 ghcr.io 的内容一致。
加速站走自己的 token 端点（`/openapi/v1/auth/token`），`docker pull` 会自动处理，无需手工配置。

## 12. 宝塔面板 / 双层 Nginx 反代（套域名）

`域名 → 宝塔 Nginx（80/443，TLS）→ 127.0.0.1:8080 → 容器内 Nginx → 静态站点 + /api → 容器内后端`

功能上没问题（就是多一跳），但要按下面几处配置，否则会踩坑。

**a) 反代配置（宝塔「网站 → 反向代理」里改）**

```nginx
location / {
    proxy_pass http://127.0.0.1:8080;
    proxy_http_version 1.1;
    proxy_set_header Host              $host;
    proxy_set_header X-Real-IP         $remote_addr;
    # ⚠️ 关键：不带这一行，所有访客在容器看来都是同一个 IP
    proxy_set_header X-Forwarded-For   $proxy_add_x_forwarded_for;
    proxy_set_header X-Forwarded-Proto $scheme;
    proxy_set_header X-Forwarded-Host  $host;
    client_max_body_size 64m;    # 必须 ≥ 32m（后端上传上限；备份导入需要更大，按需 1024m）
    proxy_read_timeout   300s;   # 备份导出/导入可能超过默认 60s，否则 504
    proxy_buffering      off;    # 流式 zip 导出建议关缓冲
}
```

**b) 三件事必须做**

1. **容器端口只绑本机**：`docker run … -p 127.0.0.1:8080:80 …`。否则 `http://<服务器IP>:8080` 可绕过 HTTPS 直连。
2. **后台把 `base_url` 设成 `https://你的域名`**（站点管理 → base_url）。推导链是
   `站点设置 → config.toml [server] base_url → 请求头`，而容器内 Nginx 会把 `X-Forwarded-Proto`
   按**容器内**的连接（http）写下去，所以不设 base_url 时 RSS / sitemap / OG 卡片 / 通知邮件里的
   链接会变成 `http://…`（甚至 `127.0.0.1:8080`）。设了 base_url 就与代理头无关，最稳。
3. **别在宝塔层开缓存 / 防盗链**：
   - 开缓存会**破坏爬虫 UA 分流**（分流在容器内 Nginx 做，但缓存按 URI 命中，爬虫和真人会拿到同一份
     内容 → 要么分享卡片失效，要么爬虫拿到 SPA）；
   - 防盗链（Referer 校验）会让 `/api/uploads/*` 图片在编辑器/后台加载失败。

**c) 其他说明**

- 访客真实 IP：后端取 `X-Forwarded-For` **首项**（最早的客户端）→ 只要按 (a) 透传就是真 IP；
  漏传时取容器内看到的上一层地址，**反滥用限流（同 IP+目标 60 秒 1 条、10 分钟 5 条）与登录失败退避
  会把所有访客当成同一个人**，正常用户被误 429 / 误锁。
- UA 分流本身不受双层代理影响（UA 原样透传），前提是 (b) 第 3 条。
- 无需 WebSocket / Upgrade 头；gzip 两层都开也无害（内层压过的不会重复压）；
  JWT 存 localStorage（不是 Cookie），所以没有 SameSite/Secure 相关配置。
- 容器 HEALTHCHECK 探测的是容器内 80 端口，与宝塔无关；宝塔侧可另配自定义监控打 `https://域名/api/site/settings`。
