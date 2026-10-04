//! 管理员资料 / 用户设置（契约「管理员资料 / 用户设置」条款，2026-10-04 新增）：
//! - GET /api/admin/profile → ProfileAdmin {username, created_at}
//! - PUT /api/admin/profile → ProfileAdmin（改用户名 / 改密码；current_password 必填）
//!
//! 单管理员模型：操作对象是 users 表首行（安装向导创建的唯一管理员）。
//! **改用户名/改密码都不会让已签发的 JWT 失效**（Bearer 校验只验签名与过期，
//! 不比对密码哈希，也不要求 token 的 sub 与当前用户名一致）——其它已登录设备保持在线；
//! 如需强制全部设备下线，请手动修改 config.toml 的 jwt_secret 并重启（绝不自动轮换）。
//!
//! 用户名/密码的格式校验与安装向导一致（trim 后非空；不另设长度/字符集强度规则），
//! 哈希与校验复用 crate::auth 的 argon2 实现，不新写一套。

use axum::extract::State;
use axum::http::HeaderMap;
use axum::Json;
use sqlx::AnyPool;
use sqlx::Row;

use crate::auth::{hash_password, verify_password};
use crate::error::{ApiError, ApiResult, ValidJson};
use crate::models::{ProfileAdmin, ProfileBody};
use crate::state::{require_pool, AppState};

use super::helpers::{check_auth, is_unique_violation};

/// 唯一管理员行（内部用；password_hash 只参与校验与回写，绝不出现在响应里）
struct AdminRow {
    id: i64,
    username: String,
    created_at: String,
    password_hash: String,
}

async fn load_admin(pool: &AnyPool) -> ApiResult<AdminRow> {
    let row = sqlx::query(
        "SELECT id, username, created_at, password_hash FROM users \
                           ORDER BY id ASC LIMIT 1",
    )
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| ApiError::internal("管理员账号不存在"))?;
    Ok(AdminRow {
        id: row.get::<i64, _>("id"),
        username: row.get::<String, _>("username"),
        created_at: row.get::<String, _>("created_at"),
        password_hash: row.get::<String, _>("password_hash"),
    })
}

fn username_taken(username: &str) -> ApiError {
    ApiError::conflict("username_taken", format!("用户名 '{username}' 已被占用"))
}

/// GET /api/admin/profile → ProfileAdmin（当前用户名与创建时间）
pub async fn admin_get_profile(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> ApiResult<Json<ProfileAdmin>> {
    check_auth(&state, &headers).await?;
    let (pool, _db_type) = require_pool(&state).await?;
    let admin = load_admin(&pool).await?;
    Ok(Json(ProfileAdmin {
        username: admin.username,
        created_at: admin.created_at,
    }))
}

/// PUT /api/admin/profile → ProfileAdmin（改用户名 / 改密码；字段可选但至少给一个）
///
/// 校验顺序（契约条款）：
/// 1. `current_password` 与当前 argon2 哈希不匹配 → 401 `invalid_credentials`，**不落库**；
/// 2. `username`/`new_password` 都缺（或 null）→ 422 `validation_error`；
/// 3. 目标用户名与安装向导同款校验（trim 后非空），唯一性冲突（排除自身）→ 409 `username_taken`；
/// 4. 新密码非空即可（不设强度规则），用现有 hash_password（argon2 + 随机盐）重新哈希。
pub async fn admin_update_profile(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: ValidJson<ProfileBody>,
) -> ApiResult<Json<ProfileAdmin>> {
    check_auth(&state, &headers).await?;
    let Json(req) = body.map_err(ApiError::from)?;
    let (pool, _db_type) = require_pool(&state).await?;
    let admin = load_admin(&pool).await?;

    // ---- 1. 当前密码必填且必须匹配（写库之前判定：失败不落库）----
    if !verify_password(&admin.password_hash, &req.current_password) {
        return Err(ApiError::invalid_credentials());
    }

    // ---- 2. 待改字段收集（与安装向导同款校验）----
    let new_username = match req.username {
        Some(u) => {
            let u = u.trim().to_string();
            if u.is_empty() {
                return Err(ApiError::validation("用户名不能为空"));
            }
            Some(u)
        }
        None => None,
    };
    let new_password = match req.new_password {
        Some(p) => {
            if p.is_empty() {
                return Err(ApiError::validation("新密码不能为空"));
            }
            Some(p)
        }
        None => None,
    };
    if new_username.is_none() && new_password.is_none() {
        return Err(ApiError::validation(
            "username 与 new_password 至少提供一个",
        ));
    }

    // ---- 3. 用户名唯一性（排除自身；与现有用户名相同视为无变化）----
    if let Some(u) = &new_username {
        if u != &admin.username {
            let taken: i64 =
                sqlx::query_scalar("SELECT COUNT(*) FROM users WHERE username = ? AND id <> ?")
                    .bind(u)
                    .bind(admin.id)
                    .fetch_one(&pool)
                    .await?;
            if taken > 0 {
                return Err(username_taken(u));
            }
        }
    }

    let username = new_username.unwrap_or_else(|| admin.username.clone());
    // 未改密码则原样回写现有哈希（不重复哈希、不改变盐）
    let password_hash = match new_password {
        Some(p) => hash_password(&p)?,
        None => admin.password_hash.clone(),
    };

    let update = sqlx::query("UPDATE users SET username = ?, password_hash = ? WHERE id = ?")
        .bind(&username)
        .bind(&password_hash)
        .bind(admin.id)
        .execute(&pool)
        .await;
    if let Err(e) = update {
        // 预检之外的并发插入兜底：唯一约束同样映射 409 username_taken
        if is_unique_violation(&e) {
            return Err(username_taken(&username));
        }
        return Err(e.into());
    }

    Ok(Json(ProfileAdmin {
        username,
        created_at: admin.created_at,
    }))
}
