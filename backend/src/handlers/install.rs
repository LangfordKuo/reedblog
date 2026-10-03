//! 安装向导：GET /api/health、GET /api/install/status、POST /api/install
//!
//! POST /api/install 流程（契约顺序）：
//! 验证连接 → 写入 config.toml → 建表（按 db_type 跑迁移）→ 创建管理员（argon2）
//! → 生成 JWT secret 存 config → 进程内切换到已安装状态（无需重启）。
//!
//! 「已安装」的判定 = config.toml 可完整加载（含非空 jwt_secret）。

use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use serde_json::{json, Value};
use sqlx::any::{AnyConnectOptions, AnyPoolOptions};
use sqlx::{AnyConnection, Connection};
use std::path::Path;
use std::time::Duration;

use crate::auth::{generate_jwt_secret, hash_password};
use crate::config::{
    AuthConfig, Config, CorsConfig, DatabaseConfig, MysqlConfig, PluginsConfig, ServerConfig,
    SiteConfig, ThemesConfig, UploadsConfig,
};
use crate::error::{ApiError, ApiResult, ValidJson};
use crate::models::InstallRequest;
use crate::state::{now_rfc3339, run_migrations, AppState};

/// GET /api/health —— 永远可用
pub async fn health() -> Json<Value> {
    Json(json!({ "status": "ok" }))
}

/// GET /api/install/status —— 永远可用
pub async fn install_status(State(state): State<AppState>) -> Json<Value> {
    Json(json!({ "installed": state.is_installed().await }))
}

/// POST /api/install
pub async fn install(
    State(state): State<AppState>,
    body: ValidJson<InstallRequest>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    if state.is_installed().await {
        return Err(ApiError::conflict(
            "already_installed",
            "站点已安装，不能重复安装",
        ));
    }
    let Json(req) = body.map_err(ApiError::from)?;

    // ---- 参数校验 ----
    if req.db_type != "sqlite" && req.db_type != "mysql" {
        return Err(ApiError::validation("db_type 必须是 sqlite 或 mysql"));
    }
    if req.db_type == "mysql" && req.mysql.is_none() {
        return Err(ApiError::validation(
            "db_type=mysql 时必须提供 mysql 连接配置",
        ));
    }
    let admin_username = req.admin.username.trim();
    if admin_username.is_empty() {
        return Err(ApiError::validation("管理员用户名不能为空"));
    }
    if req.admin.password.is_empty() {
        return Err(ApiError::validation("管理员密码不能为空"));
    }
    if req.site.title.trim().is_empty() {
        return Err(ApiError::validation("站点标题不能为空"));
    }

    let sqlite_path = req
        .sqlite_path
        .clone()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "reedblog.db".to_string());

    // ---- 组装配置（保留已有 config.toml 的 server/cors 自定义）----
    let config_path = Path::new(state.config_path()).to_path_buf();
    let mut cfg = Config::load(&config_path).unwrap_or_else(|| Config {
        server: ServerConfig::default(),
        database: DatabaseConfig {
            db_type: String::new(),
            sqlite_path: String::new(),
            mysql: None,
        },
        site: SiteConfig {
            title: String::new(),
            subtitle: None,
        },
        auth: AuthConfig {
            jwt_secret: String::new(),
        },
        cors: CorsConfig::default(),
        plugins: PluginsConfig::default(),
        themes: ThemesConfig::default(),
        uploads: UploadsConfig::default(),
    });
    cfg.database.db_type = req.db_type.clone();
    if req.db_type == "sqlite" {
        cfg.database.sqlite_path = sqlite_path;
        cfg.database.mysql = None;
    } else if let Some(m) = &req.mysql {
        cfg.database.sqlite_path = if cfg.database.sqlite_path.is_empty() {
            "reedblog.db".to_string()
        } else {
            cfg.database.sqlite_path.clone()
        };
        cfg.database.mysql = Some(MysqlConfig {
            host: m.host.clone(),
            port: m.port,
            username: m.username.clone(),
            password: m.password.clone(),
            database: m.database.clone(),
        });
    }
    cfg.site.title = req.site.title.trim().to_string();
    cfg.site.subtitle = req.site.subtitle.clone().filter(|s| !s.trim().is_empty());

    let url = cfg
        .db_url()
        .ok_or_else(|| ApiError::validation("db_type 无效或缺少 mysql 连接配置"))?;

    // ---- 1. 验证连接 ----
    let opts: AnyConnectOptions = url
        .parse()
        .map_err(|e: sqlx::Error| ApiError::validation(format!("数据库 URL 无效: {e}")))?;
    let mut probe = AnyConnection::connect_with(&opts).await.map_err(|e| {
        ApiError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "db_connection_failed",
            format!("数据库连接失败: {e}"),
        )
    })?;

    // ---- 2. 写入 config.toml（jwt_secret 同时生成写入；失败路径不 activate，进程仍处未安装态）----
    cfg.auth.jwt_secret = generate_jwt_secret();
    cfg.save(&config_path)?;

    // ---- 3. 建表（按 db_type 跑对应迁移）----
    run_migrations(&req.db_type, &mut probe)
        .await
        .map_err(|e| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "migration_failed",
                format!("数据库迁移失败: {e}"),
            )
        })?;

    // ---- 4. 创建管理员（argon2 哈希）----
    let password_hash = hash_password(&req.admin.password)?;
    let now = now_rfc3339();
    sqlx::query("INSERT INTO users (username, password_hash, created_at) VALUES (?, ?, ?)")
        .bind(admin_username)
        .bind(&password_hash)
        .bind(&now)
        .execute(&mut probe)
        .await
        .map_err(|e| ApiError::internal(format!("创建管理员失败: {e}")))?;

    // ---- 5. 建立正式连接池，进程内切换到已安装状态 ----
    drop(probe);
    let pool = AnyPoolOptions::new()
        .max_connections(if req.db_type == "sqlite" { 4 } else { 10 })
        .acquire_timeout(Duration::from_secs(10))
        .connect_with(opts)
        .await
        .map_err(|e| ApiError::internal(format!("建立连接池失败: {e}")))?;

    // 安装即「首次运行」：补建内置 default 主题 + 按 DB 恢复插件启用状态
    crate::themes::ensure_default_theme(state.themes_dir());
    state.plugins().restore_from_db(&pool).await;

    state
        .activate(
            req.db_type.clone(),
            pool,
            cfg.auth.jwt_secret.clone(),
            cfg.site.title.clone(),
            cfg.site.subtitle.clone().unwrap_or_default(),
        )
        .await;

    Ok((StatusCode::CREATED, Json(json!({ "ok": true }))))
}
