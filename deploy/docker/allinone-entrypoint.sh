#!/bin/sh
# reedblog 一体化镜像入口（POSIX sh）：Nginx 与后端进程同容器。
#
# 与后端镜像的差别只有「怎么把两个进程拉起来」；数据目录检查与未安装态 config.toml 生成
# 复用 entrypoint-lib.sh 的同一份逻辑（不是抄一份），见该文件头部注释。
#
# 启动顺序：
#   1) 准备数据目录 + 生成配置（已存在则绝不覆盖）；
#   2) `/docker-entrypoint.sh nginx -t`：Nginx 官方入口只有在第一个参数是 nginx 时才会跑
#      /docker-entrypoint.d/ 下的脚本（其中 20-envsubst-on-templates.sh 负责把
#      /etc/nginx/templates/default.conf.template 用 envsubst 渲染成 /etc/nginx/conf.d/default.conf），
#      所以这里带 nginx 参数调用一次：既完成模板渲染，又做一遍语法校验——模板/语法问题在启动阶段
#      就退出，不会留一个「只有后端在跑、80 端口没人听」的半死容器；
#   3) 后台起 nginx（-g 'daemon off;' 让 nginx 前台运行，只是被 shell 放到后台）；
#   4) exec 后端 → 后端成为 PID 1。容器停止时 PID 1 退出，内核随即清理同一 PID 命名空间里的其余
#      进程（nginx 一并被清掉），因此不需要 trap 或信号转发。两个进程谁先挂都有兜底：nginx 挂掉
#      → HEALTHCHECK 探测 80 端口失败 → 容器 unhealthy；后端挂掉 → PID 1 退出 → 容器结束。
#
# 运行身份：本镜像以 root 运行（容器内）。nginx 需要绑 80 端口、写 /var/cache/nginx 与 /var/run，
# 非 root 还要额外处理端口能力与目录属主，复杂度明显上升；一体化镜像的定位就是「一条命令先跑起来」。
# 追求进程隔离/最小权限请用 compose 的两个镜像（后端非 root，uid 10002），见 deploy/DOCKER.md。
#
# 传入命令行参数时只执行该命令（调试/测试用，不启动 nginx）；否则按上面的顺序启动整套。
set -eu

# 入口与公共片段在镜像内同目录（/usr/local/bin/），按脚本自身位置解析，不依赖当前工作目录
SCRIPT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
# shellcheck source=entrypoint-lib.sh
. "$SCRIPT_DIR/entrypoint-lib.sh"

# ---- 1&2. 数据目录检查 + 生成配置（与后端镜像同一份逻辑）----
reedblog_prepare_data_dir

if [ "$#" -gt 0 ]; then
    exec "$@"
fi

# 常见坑：改了 REEDBLOG_PORT 却忘了同步改 BACKEND_UPSTREAM（镜像里默认 127.0.0.1:3000），
# 结果 nginx 反代到一个没人监听的端口 → 全站 502。这里显式提示（不阻止启动，留出
# 「故意把 BACKEND_UPSTREAM 指向外部后端」的空间）。
if [ "${BACKEND_UPSTREAM:-}" = "127.0.0.1:3000" ] && [ "${REEDBLOG_PORT:-3000}" != "3000" ]; then
    reedblog_log "警告：REEDBLOG_PORT=${REEDBLOG_PORT} 与 BACKEND_UPSTREAM=${BACKEND_UPSTREAM} 不一致，nginx 会反代不到后端；请把 BACKEND_UPSTREAM 改为 127.0.0.1:${REEDBLOG_PORT:-3000}"
fi

# ---- 3. 渲染 nginx 模板并语法校验（失败即退出）----
if ! /docker-entrypoint.sh nginx -t; then
    reedblog_fail "nginx 配置校验失败（模板 /etc/nginx/templates/default.conf.template → /etc/nginx/conf.d/default.conf）"
fi

# ---- 4. 后台起 nginx，后端成为 PID 1 ----
reedblog_log "启动 nginx（后台，监听 80，/api 反代 → ${BACKEND_UPSTREAM:-127.0.0.1:3000}）"
/docker-entrypoint.sh nginx -g 'daemon off;' &

reedblog_log "启动后端：配置 $REEDBLOG_CONFIG，监听 0.0.0.0:${REEDBLOG_PORT:-3000}"
exec /app/reedblog-backend
