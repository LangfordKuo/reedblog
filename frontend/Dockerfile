# syntax=docker/dockerfile:1.7
# reedblog Web 镜像：node:24-alpine 构建 dist → nginx:alpine 托管静态站点 + /api 反代 + UA 爬虫分流
#
# 注意：构建上下文是**仓库根**（compose 与 CI 均如此），因为要同时 COPY frontend/ 与
# deploy/docker/nginx.conf.template。
#   docker build -t reedblog-web -f frontend/Dockerfile .
#
# 运行（BACKEND_UPSTREAM 指向后端，不带 http:// 前缀）：
#   docker run -d -p 8080:80 -e BACKEND_UPSTREAM=backend:3000 ghcr.io/langfordkuo/reedblog-web

# ---------- builder ----------
FROM node:24-alpine AS builder

WORKDIR /src
# 先只拷依赖清单，package.json 不变时 npm ci 层命中缓存
COPY frontend/package.json frontend/package-lock.json ./
RUN npm ci

COPY frontend/ ./
RUN npm run build

# ---------- runtime ----------
FROM nginx:alpine AS runtime

# 静态产物
COPY --from=builder /src/dist /usr/share/nginx/html
# 官方镜像入口会把 /etc/nginx/templates/*.template 经 envsubst 渲染到 /etc/nginx/conf.d/
# （只替换容器内已定义的环境变量；$http_user_agent、$uri 等 Nginx 变量不受影响）
COPY deploy/docker/nginx.conf.template /etc/nginx/templates/default.conf.template

# Nginx 反代目标（compose 网络内的后端服务名）；模板中拼成 http://${BACKEND_UPSTREAM}
ENV BACKEND_UPSTREAM=backend:3000

EXPOSE 80
