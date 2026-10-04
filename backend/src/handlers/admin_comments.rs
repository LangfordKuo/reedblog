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
// 嵌套评论扩展（契约「评论回复」条款）：parent_id/reply_to_id/reply_to_name（rt 自 JOIN）
// + reply_count（直接子回复条数，供后台「删除将连带删除 N 条回复」提示；子回复恒 0）。
const ADMIN_COMMENT_COLUMNS: &str = "c.id, c.post_id, c.target_type, \
     COALESCE(p.title, pg.title, '') AS post_title, \
     c.author_name, c.email, c.content, c.status, c.created_at, \
     c.parent_id, c.reply_to_id, rt.author_name AS reply_to_name, \
     (SELECT COUNT(*) FROM comments r WHERE r.parent_id = c.id) AS reply_count";

const ADMIN_COMMENT_FROM: &str = "FROM comments c \
     LEFT JOIN posts p ON p.id = c.post_id AND c.target_type = 'post' \
     LEFT JOIN pages pg ON pg.id = c.post_id AND c.target_type = 'page' \
     LEFT JOIN comments rt ON rt.id = c.reply_to_id";

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
        parent_id: r.try_get::<Option<i64>, _>("parent_id").unwrap_or(None),
        reply_to_id: r.try_get::<Option<i64>, _>("reply_to_id").unwrap_or(None),
        reply_to_name: r
            .try_get::<Option<String>, _>("reply_to_name")
            .unwrap_or(None),
        reply_count: r.try_get::<i64, _>("reply_count").unwrap_or(0),
    }
}

/// status 校验（契约「评论审核方式」2026-10-04 扩展）：approved / hidden / pending 三选一；
/// 列表过滤与 PUT 改状态共用同一处校验
fn validate_comment_status(s: &str) -> ApiResult<()> {
    if !matches!(
        s,
        super::helpers::COMMENT_STATUS_APPROVED
            | super::helpers::COMMENT_STATUS_HIDDEN
            | super::helpers::COMMENT_STATUS_PENDING
    ) {
        return Err(ApiError::validation(
            "status 必须是 approved、hidden 或 pending",
        ));
    }
    Ok(())
}

/// GET /api/admin/comments?status&post_id&page&per_page → 分页 [CommentAdmin]，created_at DESC
/// （status 过滤支持 pending，见契约「评论审核方式」）
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

    // created_at DESC（主序不变）；时间戳为秒精度，同秒行以 id DESC 兜底保证确定性
    let list_sql = format!(
        "SELECT {ADMIN_COMMENT_COLUMNS} {ADMIN_COMMENT_FROM} \
         {where_sql} ORDER BY c.created_at DESC, c.id DESC LIMIT ? OFFSET ?"
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
///
/// 连带删除（契约「评论回复」条款）：删除顶级评论时其全部子回复一并删除
/// （两级存储下所有线程成员的 parent_id 恒指顶级 id，单条 DELETE 即覆盖）；
/// 删除子回复只删自身；id 不存在 → 404。
pub async fn admin_delete_comment(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<i64>,
) -> ApiResult<StatusCode> {
    check_auth(&state, &headers).await?;
    let (pool, _db_type) = require_pool(&state).await?;

    let deleted = sqlx::query("DELETE FROM comments WHERE id = ? OR parent_id = ?")
        .bind(id)
        .bind(id)
        .execute(&pool)
        .await?
        .rows_affected();
    if deleted == 0 {
        return Err(ApiError::not_found());
    }
    Ok(StatusCode::NO_CONTENT)
}
