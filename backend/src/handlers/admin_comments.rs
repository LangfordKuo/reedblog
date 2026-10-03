//! 管理接口：评论审核（全部需要 Bearer）

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use sqlx::Row;

use crate::error::{ApiError, ApiResult, ValidJson};
use crate::models::{normalize_paging, AdminCommentsQuery, CommentAdmin, CommentStatusBody, Page};
use crate::state::{require_pool, AppState};

use super::helpers::check_auth;

// 页面功能扩展（契约「页面」条款）：评论目标可以是文章或页面（target_type 区分）。
// post_id/post_title 字段名保留、语义扩展为「目标 id / 目标标题」：
// 来源标题按 target_type 分别 LEFT JOIN posts / pages 后 COALESCE 取之。
const ADMIN_COMMENT_COLUMNS: &str = "c.id, c.post_id, c.target_type, \
     COALESCE(p.title, pg.title, '') AS post_title, \
     c.author_name, c.email, c.content, c.status, c.created_at";

const ADMIN_COMMENT_FROM: &str = "FROM comments c \
     LEFT JOIN posts p ON p.id = c.post_id AND c.target_type = 'post' \
     LEFT JOIN pages pg ON pg.id = c.post_id AND c.target_type = 'page'";

fn row_to_comment_admin(r: &sqlx::any::AnyRow) -> CommentAdmin {
    CommentAdmin {
        id: r.get::<i64, _>("id"),
        post_id: r.get::<i64, _>("post_id"),
        post_title: r.get::<String, _>("post_title"),
        author_name: r.get::<String, _>("author_name"),
        email: r.try_get::<Option<String>, _>("email").unwrap_or(None),
        content: r.get::<String, _>("content"),
        status: r.get::<String, _>("status"),
        created_at: r.get::<String, _>("created_at"),
        target_type: r
            .try_get::<String, _>("target_type")
            .unwrap_or_else(|_| "post".to_string()),
    }
}

fn validate_comment_status(s: &str) -> ApiResult<()> {
    if s != "approved" && s != "hidden" {
        return Err(ApiError::validation("status 必须是 approved 或 hidden"));
    }
    Ok(())
}

/// GET /api/admin/comments?status&post_id&page&per_page → 分页 [CommentAdmin]，created_at DESC
pub async fn admin_list_comments(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<AdminCommentsQuery>,
) -> ApiResult<Json<Page<CommentAdmin>>> {
    check_auth(&state, &headers).await?;
    let (pool, _db_type) = require_pool(&state).await?;
    let (page, per_page) = normalize_paging(q.page, q.per_page);

    let status = q.status.clone().unwrap_or_else(|| "all".to_string());
    let status_param: Option<String> = if status == "all" {
        None
    } else {
        validate_comment_status(&status)?;
        Some(status)
    };

    let mut where_parts: Vec<&str> = Vec::new();
    if status_param.is_some() {
        where_parts.push("c.status = ?");
    }
    if q.post_id.is_some() {
        // post_id 过滤仅匹配来源为文章的评论（契约「评论」条款 2026-10-03 扩展）
        where_parts.push("c.target_type = 'post' AND c.post_id = ?");
    }
    let where_sql = if where_parts.is_empty() {
        String::new()
    } else {
        format!("WHERE {}", where_parts.join(" AND "))
    };

    let count_sql = format!("SELECT COUNT(*) {ADMIN_COMMENT_FROM} {where_sql}");
    let mut cq = sqlx::query(&count_sql);
    if let Some(s) = &status_param {
        cq = cq.bind(s.clone());
    }
    if let Some(pid) = q.post_id {
        cq = cq.bind(pid);
    }
    let total: i64 = cq.fetch_one(&pool).await?.get(0);

    let list_sql = format!(
        "SELECT {ADMIN_COMMENT_COLUMNS} {ADMIN_COMMENT_FROM} \
         {where_sql} ORDER BY c.created_at DESC LIMIT ? OFFSET ?"
    );
    let mut lq = sqlx::query(&list_sql);
    if let Some(s) = &status_param {
        lq = lq.bind(s.clone());
    }
    if let Some(pid) = q.post_id {
        lq = lq.bind(pid);
    }
    let rows = lq
        .bind(per_page)
        .bind((page - 1) * per_page)
        .fetch_all(&pool)
        .await?;

    Ok(Json(Page {
        items: rows.iter().map(row_to_comment_admin).collect(),
        total,
        page,
        per_page,
    }))
}

/// PUT /api/admin/comments/:id body {status} → CommentAdmin
pub async fn admin_update_comment(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<i64>,
    req_body: ValidJson<CommentStatusBody>,
) -> ApiResult<Json<CommentAdmin>> {
    check_auth(&state, &headers).await?;
    let Json(body) = req_body.map_err(ApiError::from)?;
    let (pool, _db_type) = require_pool(&state).await?;

    let status = body.status.trim().to_string();
    validate_comment_status(&status)?;

    let updated = sqlx::query("UPDATE comments SET status = ? WHERE id = ?")
        .bind(&status)
        .bind(id)
        .execute(&pool)
        .await?
        .rows_affected();
    if updated == 0 {
        return Err(ApiError::not_found());
    }

    let sql = format!("SELECT {ADMIN_COMMENT_COLUMNS} {ADMIN_COMMENT_FROM} WHERE c.id = ?");
    let row = sqlx::query(&sql).bind(id).fetch_one(&pool).await?;
    Ok(Json(row_to_comment_admin(&row)))
}

/// DELETE /api/admin/comments/:id → 204
pub async fn admin_delete_comment(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<i64>,
) -> ApiResult<StatusCode> {
    check_auth(&state, &headers).await?;
    let (pool, _db_type) = require_pool(&state).await?;

    let deleted = sqlx::query("DELETE FROM comments WHERE id = ?")
        .bind(id)
        .execute(&pool)
        .await?
        .rows_affected();
    if deleted == 0 {
        return Err(ApiError::not_found());
    }
    Ok(StatusCode::NO_CONTENT)
}
