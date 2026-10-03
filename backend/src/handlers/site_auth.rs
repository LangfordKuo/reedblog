//! GET /api/site、POST /api/auth/login、GET /api/auth/me

use axum::extract::State;
use axum::http::HeaderMap;
use axum::Json;
use serde_json::{json, Value};
use sqlx::Row;

use crate::auth::{issue_token, require_auth, verify_password};
use crate::error::{ApiError, ApiResult, ValidJson};
use crate::models::{AuthResult, LoginRequest, SiteInfo};
use crate::state::{require_pool, AppState};

/// GET /api/site → SiteInfo（title/subtitle 与站点设置一致，契约「站点设置-联动读取」条款；
/// 库不可读时回退 config.toml [site] 的 Runtime 缓存值）
pub async fn site_info(State(state): State<AppState>) -> Json<SiteInfo> {
    let rt = state.runtime().await;
    let (title, subtitle) = match &rt.pool {
        Some(pool) if rt.installed => match crate::settings::load(pool, &state).await {
            Ok(s) => (s.title, s.subtitle),
            Err(_) => (rt.site_title.clone(), rt.site_subtitle.clone()),
        },
        _ => (rt.site_title.clone(), rt.site_subtitle.clone()),
    };
    Json(SiteInfo {
        title,
        subtitle,
        installed: rt.installed,
    })
}

/// GET /api/site/stats → SiteStats（契约「站点设置」2026-10-03 组件系统新增）：
/// published 文章数 / approved 评论数（含页面留言）/ 安装时间（users 表最早 created_at，
/// 取不到时空串）。站点信息组件数据源；不进未安装门禁白名单，未安装 → 503。
pub async fn site_stats(State(state): State<AppState>) -> ApiResult<Json<Value>> {
    let (pool, _db_type) = require_pool(&state).await?;
    let post_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM posts WHERE status = 'published'")
            .fetch_one(&pool)
            .await?;
    let comment_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM comments WHERE status = 'approved'")
            .fetch_one(&pool)
            .await?;
    let installed_at: String =
        sqlx::query_scalar("SELECT COALESCE(MIN(created_at), '') FROM users")
            .fetch_one(&pool)
            .await?;
    Ok(Json(json!({
        "post_count": post_count,
        "comment_count": comment_count,
        "installed_at": installed_at,
    })))
}

/// POST /api/auth/login → AuthResult；用户名或密码错误 → 401 invalid_credentials
pub async fn login(
    State(state): State<AppState>,
    body: ValidJson<LoginRequest>,
) -> ApiResult<Json<AuthResult>> {
    let Json(req) = body.map_err(ApiError::from)?;
    let (pool, _db_type) = require_pool(&state).await?;

    let row = sqlx::query("SELECT username, password_hash FROM users WHERE username = ?")
        .bind(req.username.trim())
        .fetch_optional(&pool)
        .await?;

    // 先取出字段再校验，统一失败路径（避免暴露用户是否存在）
    let stored = row.map(|r| {
        (
            r.get::<String, _>("username"),
            r.get::<String, _>("password_hash"),
        )
    });
    let (username, password_hash) = match stored {
        Some(v) => v,
        None => return Err(ApiError::invalid_credentials()),
    };
    if !verify_password(&password_hash, &req.password) {
        return Err(ApiError::invalid_credentials());
    }

    let rt = state.runtime().await;
    let secret = rt
        .jwt_secret
        .clone()
        .ok_or_else(|| ApiError::internal("JWT secret 未配置"))?;
    let (token, expires_at) = issue_token(&secret, &username)?;

    Ok(Json(AuthResult {
        token,
        username,
        expires_at,
    }))
}

/// GET /api/auth/me（Bearer）→ {"username"}；无效/过期 → 401 unauthorized
pub async fn me(State(state): State<AppState>, headers: HeaderMap) -> ApiResult<Json<Value>> {
    let rt = state.runtime().await;
    let claims = require_auth(rt.jwt_secret, &headers)?;
    Ok(Json(json!({ "username": claims.sub })))
}
