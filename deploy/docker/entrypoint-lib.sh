#!/bin/sh
# reedblog 容器入口公共片段（POSIX sh）——只定义函数，source 时无副作用。
#
# 存在的意义：把「数据目录检查 + 未安装态 config.toml 生成」这段逻辑收敛成唯一一份，
# 两个入口 source 同一个文件，避免各抄一份后逐渐漂移：
#   - deploy/docker/entrypoint.sh           后端镜像（非 root，只跑后端进程）
#   - deploy/docker/allinone-entrypoint.sh  一体化镜像（nginx + 后端同容器）
# 两个镜像都把本文件与入口脚本拷到 /usr/local/bin/，入口脚本按自身所在目录 source。
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
# 调用约定：caller 先 `set -eu`，source 本文件后调用 reedblog_prepare_data_dir；
# 函数返回时 REEDBLOG_CONFIG 已 export 为最终使用的配置路径。

reedblog_log() { echo "[entrypoint] $*"; }

reedblog_fail() {
    echo "[entrypoint] 错误: $*" >&2
    exit 1
}

# TOML basic string 转义（\ 与 "）：MySQL 密码等可能含特殊字符
reedblog_toml_escape() {
    printf '%s' "$1" | sed -e 's/\\/\\\\/g' -e 's/"/\\"/g'
}

# 端口合法性：非法值会让 TOML 反序列化失败，后端静默回退到默认 127.0.0.1:3000（容器外不可达），
# 难排查，故在入口直接拒绝
reedblog_is_port() {
    case "$1" in
        '' | *[!0-9]*) return 1 ;;
    esac
    [ "${#1}" -le 5 ] || return 1
    [ "$1" -ge 1 ] && [ "$1" -le 65535 ]
}

# 数据目录检查 + 生成未安装态配置（已存在则绝不覆盖）。
# 成功后 export REEDBLOG_CONFIG。
reedblog_prepare_data_dir() {
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

    # ---- 1. 数据目录检查 ----
    [ -d "$DATA_DIR" ] || reedblog_fail "数据目录 $DATA_DIR 不存在；请把卷挂载到该目录（docker run -v reedblog-data:/data ...）"
    [ -w "$DATA_DIR" ] || reedblog_fail "数据目录 $DATA_DIR 不可写。后端镜像内以 uid 10002(reedblog) 运行，绑定挂载宿主机目录时请先 chown -R 10002:10002 <宿主机目录>（一体化镜像以 root 运行，遇到这条多为挂载只读）"

    case "$DB_TYPE" in
        sqlite | mysql) ;;
        *) reedblog_fail "REEDBLOG_DB_TYPE 只能是 sqlite 或 mysql（当前: $DB_TYPE）" ;;
    esac
    reedblog_is_port "$PORT" || reedblog_fail "REEDBLOG_PORT 必须是 1-65535 的整数（当前: $PORT）"
    if [ "$DB_TYPE" = "mysql" ]; then
        reedblog_is_port "$MYSQL_PORT" || reedblog_fail "REEDBLOG_MYSQL_PORT 必须是 1-65535 的整数（当前: $MYSQL_PORT）"
    fi

    # 数据相关目录统一落在卷内；后端自身也会按需创建，这里预建保证属主正确
    mkdir -p "$DATA_DIR/plugins" "$DATA_DIR/themes" "$DATA_DIR/uploads"
    if [ "$DB_TYPE" = "sqlite" ]; then
        mkdir -p "$(dirname "$SQLITE_PATH")"
    fi

    # ---- 2. 生成配置（仅当不存在）----
    if [ -f "$CONFIG_PATH" ]; then
        reedblog_log "检测到已有配置 $CONFIG_PATH，保持原样（入口脚本不会覆盖已有配置）"
    else
        reedblog_log "未找到 $CONFIG_PATH，生成未安装态配置（jwt_secret 留空 → 首次访问走 /install 向导）"

        # CORS 白名单：逗号分隔 → TOML 数组字面量
        cors_list=""
        set -f # 关闭路径展开，避免白名单里的通配字符被 glob
        IFS=','
        for origin in ${REEDBLOG_CORS_ORIGINS:-}; do
            origin="$(printf '%s' "$origin" | sed -e 's/^[[:space:]]*//' -e 's/[[:space:]]*$//')"
            [ -n "$origin" ] || continue
            [ -z "$cors_list" ] || cors_list="$cors_list, "
            cors_list="$cors_list\"$(reedblog_toml_escape "$origin")\""
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
sqlite_path = "$(reedblog_toml_escape "$SQLITE_PATH")"
EOF

            if [ "$DB_TYPE" = "mysql" ]; then
                cat <<EOF

[database.mysql]
host = "$(reedblog_toml_escape "$MYSQL_HOST")"
port = ${MYSQL_PORT}
username = "$(reedblog_toml_escape "$MYSQL_USER")"
password = "$(reedblog_toml_escape "$MYSQL_PASSWORD")"
database = "$(reedblog_toml_escape "$MYSQL_DATABASE")"
EOF
            fi

            cat <<EOF

[cors]
allowed_origins = [${cors_list}]

[plugins]
dir = "$(reedblog_toml_escape "$DATA_DIR/plugins")"

[themes]
dir = "$(reedblog_toml_escape "$DATA_DIR/themes")"
active = "default"

[uploads]
dir = "$(reedblog_toml_escape "$DATA_DIR/uploads")"
max_size_mb = 10
EOF
        } >"$tmp" || reedblog_fail "写入临时配置 $tmp 失败"
        mv "$tmp" "$CONFIG_PATH" || reedblog_fail "替换 $CONFIG_PATH 失败"
        reedblog_log "已生成 $CONFIG_PATH（数据库类型: $DB_TYPE）"
    fi

    export REEDBLOG_CONFIG="$CONFIG_PATH"
}
