//! 管理接口：评论审核（全部需要 Bearer）

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use sqlx::Row;

use crate::error::{ApiError, ApiResult, ValidJson};
use crate::models::{normalize_paging, AdminCommentsQuery, CommentAdmin, CommentStatusBody, Page};
use crate::state::{require_pool, AppState};

use super::helpers::check_auth;

const ADMIN_COMMENT_COLUMNS: &str =
    "c.id, c.post_id, p.title AS post_title, c.author_name, c.email, c.content, c.status, c.created_at";

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
        where_parts.push("c.post_id = ?");
    }
    let where_sql = if where_parts.is_empty() {
        String::new()
    } else {
        format!("WHERE {}", where_parts.join(" AND "))
    };

    let count_sql = format!(
        "SELECT COUNT(*) FROM comments c JOIN posts p ON p.id = c.post_id {where_sql}"
    );
    let mut cq = sqlx::query(&count_sql);
    if let Some(s) = &status_param {
        cq = cq.bind(s.clone());
    }
    if let Some(pid) = q.post_id {
        cq = cq.bind(pid);
    }
    let total: i64 = cq.fetch_one(&pool).await?.get(0);

    let list_sql = format!(
        "SELECT {ADMIN_COMMENT_COLUMNS} FROM comments c JOIN posts p ON p.id = c.post_id \
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

    let sql = format!(
        "SELECT {ADMIN_COMMENT_COLUMNS} FROM comments c JOIN posts p ON p.id = c.post_id \
         WHERE c.id = ?"
    );
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
