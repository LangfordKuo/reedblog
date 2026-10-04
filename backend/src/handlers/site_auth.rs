//! GET /api/site、POST /api/auth/login、GET /api/auth/me

use axum::extract::{ConnectInfo, State};
use axum::http::HeaderMap;
use axum::Json;
use serde_json::{json, Value};
use sqlx::Row;
use std::net::SocketAddr;

use crate::auth::{issue_token, require_auth, verify_password};
use crate::error::{ApiError, ApiResult, ValidJson};
use crate::models::{AuthResult, LoginRequest, SiteInfo};
use crate::state::{require_pool, AppState};
use crate::views::client_ip;

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

/// GET /api/site/stats → SiteStats（契约「站点设置」2026-10-03 组件系统新增；
/// total_views 为 2026-10-04「浏览量与点赞」新增）：
/// 公开可见文章数（published + 到点的 scheduled，契约「文章置顶与定时发布」可见性口径）/
/// approved 评论数（含页面留言；pending/hidden 均不计入——契约「评论审核方式」）/
/// 安装时间（users 表最早 created_at，取不到时空串）/
/// 所有文章浏览量之和（posts 全表 SUM，含草稿——历史累计口径）。
/// 站点信息组件数据源；不进未安装门禁白名单，未安装 → 503。
pub async fn site_stats(State(state): State<AppState>) -> ApiResult<Json<Value>> {
    let (pool, _db_type) = require_pool(&state).await?;
    let post_sql = format!(
        "SELECT COUNT(*) FROM posts p WHERE {}",
        crate::handlers::helpers::VISIBLE_POST_SQL
    );
    let post_count: i64 = sqlx::query_scalar(&post_sql)
        .bind(crate::state::now_rfc3339())
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
    let total_views: i64 = sqlx::query_scalar("SELECT COALESCE(SUM(view_count), 0) FROM posts")
        .fetch_one(&pool)
        .await?;
    Ok(Json(json!({
        "post_count": post_count,
        "comment_count": comment_count,
        "installed_at": installed_at,
        "total_views": total_views,
    })))
}

/// POST /api/auth/login → AuthResult；用户名或密码错误 → 401 invalid_credentials
///
/// 反滥用失败退避（契约「反滥用」条款）：同 IP + 用户名连续失败 5 次 → 锁定 15 分钟，
/// 锁定期间**即使密码正确也拒绝** → 429 too_many_attempts + Retry-After；成功登录清零计数。
/// 判定在密码校验之前（先于任何凭据比对），用户名不存在同样计入失败（不泄露用户是否存在）。
pub async fn login(
    State(state): State<AppState>,
    headers: HeaderMap,
    // 限流键的直连 IP 兜底来源；测试路径的裸 axum::serve 不提供（同评论接口）
    connect: Result<ConnectInfo<SocketAddr>, axum::extract::rejection::ExtensionRejection>,
    body: ValidJson<LoginRequest>,
) -> ApiResult<Json<AuthResult>> {
    let Json(req) = body.map_err(ApiError::from)?;
    let connect = connect.ok();
    let (pool, _db_type) = require_pool(&state).await?;

    let username = req.username.trim().to_string();
    let ip = client_ip(&headers, connect.as_ref());
    // 退避键 = "{ip}|{username}"（契约「反滥用」存储表）
    let key = format!("{ip}|{username}");
    // 锁定判定：先于密码校验（锁定期间即使密码正确也拒绝）
    if let Some(retry_after) = state.antispam().login_locked_for(&key) {
        return Err(ApiError::too_many_attempts(retry_after));
    }

    let row = sqlx::query("SELECT username, password_hash FROM users WHERE username = ?")
        .bind(&username)
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
        None => {
            state.antispam().record_login_failure(&key);
            return Err(ApiError::invalid_credentials());
        }
    };
    if !verify_password(&password_hash, &req.password) {
        state.antispam().record_login_failure(&key);
        return Err(ApiError::invalid_credentials());
    }
    // 成功清零该 IP + 用户名的失败计数（契约「反滥用」）
    state.antispam().clear_login_failures(&key);

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
