# syntax=docker/dockerfile:1.7
# reedblog 后端镜像（只跑后端进程；前端 dist 由 reedblog-web 镜像的 Nginx 托管）
#
# 构建（构建上下文 = 仓库根）：
#   docker build -t reedblog-backend .
# 运行：
#   docker run -d -p 3000:3000 -v reedblog-data:/data ghcr.io/langfordkuo/reedblog-backend
#
# 数据全部落在 /data 卷：config.toml、SQLite 文件、uploads/、plugins/、themes/
# 首次启动由 deploy/docker/entrypoint.sh 生成未安装态配置，浏览器走 /install 向导完成安装。

# ---------- builder ----------
FROM rust:1-bookworm AS builder

WORKDIR /build
# .dockerignore 已排除 backend/target、config.toml、*.db 等运行时数据
COPY backend/ ./

# cargo 依赖用 BuildKit cache mount 跨构建复用（CI 加速的关键）。
# target/ 是 cache mount，构建产物必须在本 RUN 内拷出到普通路径，才能进 runtime 层。
RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,target=/build/target,sharing=locked \
    cargo build --release --locked \
    && cp target/release/reedblog-backend /build/reedblog-backend

# ---------- runtime ----------
# bookworm = glibc 2.36，满足 gnu 产物要求的 2.35；不用 alpine/musl（sqlx bundled SQLite 走 gnu 工具链）
FROM debian:bookworm-slim AS runtime

# ca-certificates：外呼 HTTPS（如 SMTP）；curl：HEALTHCHECK 探测
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/*

# 非 root 运行：固定 uid/gid 10002。
# /data 预建为该属主——空命名卷首次挂载会继承镜像内目录的属主，避免「卷属主 root，容器内写不进」
RUN groupadd --gid 10002 reedblog \
    && useradd --uid 10002 --gid 10002 --no-create-home --shell /usr/sbin/nologin reedblog \
    && install -d -o reedblog -g reedblog -m 0755 /data

COPY --from=builder /build/reedblog-backend /app/reedblog-backend
COPY deploy/docker/entrypoint.sh /usr/local/bin/entrypoint.sh
# 显式置执行位，不依赖构建上下文的文件权限（Windows 检出的仓库可能丢失 +x）
RUN chmod 0755 /app/reedblog-backend /usr/local/bin/entrypoint.sh

WORKDIR /app
ENV REEDBLOG_CONFIG=/data/config.toml
VOLUME ["/data"]
EXPOSE 3000
USER reedblog

# 未安装态（GET /api/site/settings）返回 503 也算健康——安装前站点就是「未安装」，
# 200（已安装）与 503 之外一律不健康。REEDBLOG_PORT 改端口时这里跟随。
HEALTHCHECK --interval=10s --timeout=5s --start-period=10s --retries=3 \
    CMD curl -sS -o /dev/null -w '%{http_code}' "http://127.0.0.1:${REEDBLOG_PORT:-3000}/api/site/settings" | grep -qE '^(200|503)$' || exit 1

ENTRYPOINT ["/usr/local/bin/entrypoint.sh"]
