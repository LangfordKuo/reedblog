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

/// GET /api/site → SiteInfo
pub async fn site_info(State(state): State<AppState>) -> Json<SiteInfo> {
    let rt = state.runtime().await;
    Json(SiteInfo {
        title: rt.site_title,
        subtitle: rt.site_subtitle,
        installed: rt.installed,
    })
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
