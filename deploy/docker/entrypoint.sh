#!/bin/sh
# reedblog 后端容器入口（POSIX sh）
#
# 职责：检查/准备数据目录 → 首次运行时生成「未安装态」config.toml → exec 后端进程。
# 已存在的 config.toml 绝不覆盖（jwt_secret 与数据库配置都在里面，覆盖会丢站点数据）。
#
# 这两步的具体逻辑在 entrypoint-lib.sh 里，与一体化镜像（allinone-entrypoint.sh）共用同一份实现；
# 环境变量、行为约定见该文件头部注释与 deploy/DOCKER.md。
#
# 传入命令行参数时执行该命令（调试/测试用），否则启动 /app/reedblog-backend。
set -eu

# 入口与公共片段在镜像内同目录（/usr/local/bin/），按脚本自身位置解析，不依赖当前工作目录
SCRIPT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
# shellcheck source=entrypoint-lib.sh
. "$SCRIPT_DIR/entrypoint-lib.sh"

# ---- 1. 数据目录检查 + 2. 生成配置（公共逻辑，见 entrypoint-lib.sh）----
reedblog_prepare_data_dir

# ---- 3. 启动后端 ----
reedblog_log "启动后端：配置 $REEDBLOG_CONFIG，监听 0.0.0.0:${REEDBLOG_PORT:-3000}"
if [ "$#" -gt 0 ]; then
    exec "$@"
fi
exec /app/reedblog-backend
