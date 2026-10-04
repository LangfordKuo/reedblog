#!/bin/sh
# reedblog 后端容器入口（POSIX sh）
#
# 职责：检查/准备数据目录 → 首次运行时生成「未安装态」config.toml → exec 后端进程。
# 已存在的 config.toml 绝不覆盖（jwt_secret 与数据库配置都在里面，覆盖会丢站点数据）。
#
# 环境变量（全部有默认值，详见 deploy/DOCKER.md）：
#   REEDBLOG_CONFIG        配置文件路径，默认 /data/config.toml
#   REEDBLOG_PORT          [server] port，默认 3000（[server] host 固定 0.0.0.0）
#   REEDBLOG_DB_TYPE       sqlite | mysql，默认 sqlite
#   REEDBLOG_SQLITE_PATH   SQLite 文件路径，默认 /data/reedblog.db
#   REEDBLOG_MYSQL_HOST / _PORT / _USER / _PASSWORD / _DATABASE
#                          MySQL 连接（默认 host=mysql、port=3306，与 compose 的 mysql 服务同名）
#   REEDBLOG_CORS_ORIGINS  逗号分隔的跨域白名单，默认空（同源反代不需要 CORS）
#
# 传入命令行参数时执行该命令（调试/测试用），否则启动 /app/reedblog-backend。
set -eu

CONFIG_PATH="${REEDBLOG_CONFIG:-/data/config.toml}"
DATA_DIR="$(dirname "$CONFIG_PATH")"
PORT="${REEDBLOG_PORT:-3000}"
DB_TYPE="${REEDBLOG_DB_TYPE:-sqlite}"
SQLITE_PATH="${REEDBLOG_SQLITE_PATH:-/data/reedblog.db}"
MYSQL_HOST="${REEDBLOG_MYSQL_HOST:-mysql}"
MYSQL_PORT="${REEDBLOG_MYSQL_PORT:-3306}"
MYSQL_USER="${REEDBLOG_MYSQL_USER:-reedblog}"
MYSQL_PASSWORD="${REEDBLOG_MYSQL_PASSWORD:-}"
MYSQL_DATABASE="${REEDBLOG_MYSQL_DATABASE:-reedblog}"

log() { echo "[entrypoint] $*"; }
fail() {
    echo "[entrypoint] 错误: $*" >&2
    exit 1
}

# TOML basic string 转义（\ 与 "）：MySQL 密码等可能含特殊字符
toml_escape() {
    printf '%s' "$1" | sed -e 's/\\/\\\\/g' -e 's/"/\\"/g'
}

# 端口合法性：非法值会让 TOML 反序列化失败，后端静默回退到默认 127.0.0.1:3000（容器外不可达），
# 难排查，故在入口直接拒绝
is_port() {
    case "$1" in
        '' | *[!0-9]*) return 1 ;;
    esac
    [ "${#1}" -le 5 ] || return 1
    [ "$1" -ge 1 ] && [ "$1" -le 65535 ]
}

# ---- 1. 数据目录检查 ----
[ -d "$DATA_DIR" ] || fail "数据目录 $DATA_DIR 不存在；请把卷挂载到该目录（docker run -v reedblog-data:/data ...）"
[ -w "$DATA_DIR" ] || fail "数据目录 $DATA_DIR 不可写。容器内以 uid 10002(reedblog) 运行，绑定挂载宿主机目录时请先 chown -R 10002:10002 <宿主机目录>"

case "$DB_TYPE" in
    sqlite | mysql) ;;
    *) fail "REEDBLOG_DB_TYPE 只能是 sqlite 或 mysql（当前: $DB_TYPE）" ;;
esac
is_port "$PORT" || fail "REEDBLOG_PORT 必须是 1-65535 的整数（当前: $PORT）"
if [ "$DB_TYPE" = "mysql" ]; then
    is_port "$MYSQL_PORT" || fail "REEDBLOG_MYSQL_PORT 必须是 1-65535 的整数（当前: $MYSQL_PORT）"
fi

# 数据相关目录统一落在卷内；后端自身也会按需创建，这里预建保证属主正确
mkdir -p "$DATA_DIR/plugins" "$DATA_DIR/themes" "$DATA_DIR/uploads"
if [ "$DB_TYPE" = "sqlite" ]; then
    mkdir -p "$(dirname "$SQLITE_PATH")"
fi

# ---- 2. 生成配置（仅当不存在）----
if [ -f "$CONFIG_PATH" ]; then
    log "检测到已有配置 $CONFIG_PATH，保持原样（入口脚本不会覆盖已有配置）"
else
    log "未找到 $CONFIG_PATH，生成未安装态配置（jwt_secret 留空 → 首次访问走 /install 向导）"

    # CORS 白名单：逗号分隔 → TOML 数组字面量
    cors_list=""
    set -f # 关闭路径展开，避免白名单里的通配字符被 glob
    IFS=','
    for origin in ${REEDBLOG_CORS_ORIGINS:-}; do
        origin="$(printf '%s' "$origin" | sed -e 's/^[[:space:]]*//' -e 's/[[:space:]]*$//')"
        [ -n "$origin" ] || continue
        [ -z "$cors_list" ] || cors_list="$cors_list, "
        cors_list="$cors_list\"$(toml_escape "$origin")\""
    done
    set +f
    unset IFS

    # 先写临时文件再原子替换：中途失败不会留下半份配置（半份=解析失败=未安装态）
    tmp="${CONFIG_PATH}.tmp.$$"
    {
        cat <<EOF
# reedblog 配置（由容器入口在首次启动时生成）
#
# 本文件只在不存在时生成，入口脚本不会覆盖已有配置。
# jwt_secret 为空 = 未安装态：浏览器首次访问会进入 /install 安装向导，
# 向导完成后把完整配置（含自动生成的 jwt_secret）写回本文件，重启不丢。
# 手工修改本文件后需重启容器生效。

[server]
host = "0.0.0.0"
port = ${PORT}
base_url = ""

[auth]
jwt_secret = ""

[database]
db_type = "${DB_TYPE}"
sqlite_path = "$(toml_escape "$SQLITE_PATH")"
EOF

        if [ "$DB_TYPE" = "mysql" ]; then
            cat <<EOF

[database.mysql]
host = "$(toml_escape "$MYSQL_HOST")"
port = ${MYSQL_PORT}
username = "$(toml_escape "$MYSQL_USER")"
password = "$(toml_escape "$MYSQL_PASSWORD")"
database = "$(toml_escape "$MYSQL_DATABASE")"
EOF
        fi

        cat <<EOF

[cors]
allowed_origins = [${cors_list}]

[plugins]
dir = "$(toml_escape "$DATA_DIR/plugins")"

[themes]
dir = "$(toml_escape "$DATA_DIR/themes")"
active = "default"

[uploads]
dir = "$(toml_escape "$DATA_DIR/uploads")"
max_size_mb = 10
EOF
    } >"$tmp" || fail "写入临时配置 $tmp 失败"
    mv "$tmp" "$CONFIG_PATH" || fail "替换 $CONFIG_PATH 失败"
    log "已生成 $CONFIG_PATH（数据库类型: $DB_TYPE）"
fi

# ---- 3. 启动后端 ----
export REEDBLOG_CONFIG="$CONFIG_PATH"
log "启动后端：配置 $CONFIG_PATH，监听 0.0.0.0:$PORT"
if [ "$#" -gt 0 ]; then
    exec "$@"
fi
exec /app/reedblog-backend
