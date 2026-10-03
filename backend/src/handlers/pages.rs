//! 页面公开接口（契约「页面」条款，2026-10-03 新增）：
//! - GET /api/pages            → [PageSummary]（仅 enabled；前台顶栏导航数据源）
//! - GET /api/pages/:slug      → PageDetail（停用/不存在 404；content_html 走文章同款钩子管线实时渲染）
//! - GET/POST /api/pages/:slug/comments → 留言板留言（仅 kind=message_board 的启用页面；
//!   留言 = target_type='page' 的评论，复用评论管线：先发后审 + comment.before_create 钩子）

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use sqlx::AnyPool;
use sqlx::Row;

use crate::error::{ApiError, ApiResult, ValidJson};
use crate::models::{CommentPub, CreateCommentRequest, PageDetail, PageSummary};
use crate::pages::{
    fetch_page_links, row_to_page_summary, KIND_LINKS, KIND_MESSAGE_BOARD, PAGE_COLUMNS,
};
use crate::plugins::CommentDecision;
use crate::state::{now_rfc3339, require_pool, AppState};

use super::helpers::{last_insert_id_on, render_markdown};

/// GET /api/pages → [PageSummary]（仅 enabled，sort_order ASC, id ASC）
pub async fn list_pages(State(state): State<AppState>) -> ApiResult<Json<Vec<PageSummary>>> {
    let (pool, _db_type) = require_pool(&state).await?;
    let rows = sqlx::query(
        "SELECT id, title, slug, kind, sort_order FROM pages \
         WHERE enabled = 1 ORDER BY sort_order ASC, id ASC",
    )
    .fetch_all(&pool)
    .await?;
    Ok(Json(rows.iter().map(row_to_page_summary).collect()))
}

/// GET /api/pages/:slug → PageDetail；不存在/停用 → 404 not_found
///
/// content_html 渲染管线与文章详情同款（契约「页面」条款）：
/// post.before_render 链改写 content_md → Markdown 渲染 → post.after_render 链改写。
/// kind=links 时附带链接列表（sort_order ASC），其余 kind 恒为 []。
pub async fn get_page(
    State(state): State<AppState>,
    Path(slug): Path<String>,
) -> ApiResult<Json<PageDetail>> {
    let (pool, _db_type) = require_pool(&state).await?;
    let sql = format!("SELECT {PAGE_COLUMNS} FROM pages WHERE slug = ? AND enabled = 1");
    let row = sqlx::query(&sql)
        .bind(&slug)
        .fetch_optional(&pool)
        .await?
        .ok_or_else(ApiError::not_found)?;

    let id = row.get::<i64, _>("id");
    let title = row.get::<String, _>("title");
    let kind = row.get::<String, _>("kind");
    let content_md = row.get::<String, _>("content_md");

    let (render_title, render_md) = state
        .plugins()
        .run_post_before_render(&title, &content_md, &slug)
        .await;
    let raw_html = render_markdown(&render_md);
    let content_html = state
        .plugins()
        .run_post_after_render(&render_title, &raw_html, &slug)
        .await;

    let links = if kind == KIND_LINKS {
        fetch_page_links(&pool, id).await?
    } else {
        Vec::new()
    };

    Ok(Json(PageDetail {
        id,
        title,
        slug,
        kind,
        content_html,
        sort_order: row.get::<i64, _>("sort_order"),
        updated_at: row.get::<String, _>("updated_at"),
        links,
    }))
}

/// 找启用中的留言板页 id（留言读写共用）；
/// 不存在/停用/kind 非 message_board 一律 404（契约「页面」条款）
async fn find_message_board_page(pool: &AnyPool, slug: &str) -> ApiResult<i64> {
    let row = sqlx::query("SELECT id FROM pages WHERE slug = ? AND enabled = 1 AND kind = ?")
        .bind(slug)
        .bind(KIND_MESSAGE_BOARD)
        .fetch_optional(pool)
        .await?;
    Ok(row.ok_or_else(ApiError::not_found)?.get::<i64, _>("id"))
}

/// GET /api/pages/:slug/comments → [CommentPub]（仅 approved，时间 ASC）
pub async fn list_page_comments(
    State(state): State<AppState>,
    Path(slug): Path<String>,
) -> ApiResult<Json<Vec<CommentPub>>> {
    let (pool, _db_type) = require_pool(&state).await?;
    let page_id = find_message_board_page(&pool, &slug).await?;
    let rows = sqlx::query(
        "SELECT id, author_name, content, created_at FROM comments \
         WHERE target_type = 'page' AND post_id = ? AND status = 'approved' \
         ORDER BY created_at ASC",
    )
    .bind(page_id)
    .fetch_all(&pool)
    .await?;
    Ok(Json(
        rows.iter()
            .map(|r| CommentPub {
                id: r.get::<i64, _>("id"),
                author_name: r.get::<String, _>("author_name"),
                content: r.get::<String, _>("content"),
                created_at: r.get::<String, _>("created_at"),
            })
            .collect(),
    ))
}

/// POST /api/pages/:slug/comments → 201 CommentPub（先发后审：创建即 approved）
///
/// 与文章评论同一条管线：必填校验 → comment.before_create 钩子链（block → 403
/// comment_blocked；ctx.post_slug = 页面 slug）→ 插入 target_type='page' 的评论行。
pub async fn create_page_comment(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    body: ValidJson<CreateCommentRequest>,
) -> ApiResult<(StatusCode, Json<CommentPub>)> {
    let Json(req) = body.map_err(ApiError::from)?;
    let (pool, db_type) = require_pool(&state).await?;
    let page_id = find_message_board_page(&pool, &slug).await?;

    let author_name = req.author_name.trim();
    let content = req.content.trim();
    if author_name.is_empty() {
        return Err(ApiError::validation("author_name 不能为空"));
    }
    if content.is_empty() {
        return Err(ApiError::validation("content 不能为空"));
    }
    let email = req
        .email
        .map(|e| e.trim().to_string())
        .filter(|e| !e.is_empty());

    let (author_name, email, content) = match state
        .plugins()
        .run_comment_before_create(&slug, author_name, email.as_deref(), content)
        .await
    {
        CommentDecision::Block { reason } => {
            let message = if reason.trim().is_empty() {
                "留言被插件拦截".to_string()
            } else {
                reason
            };
            return Err(ApiError::new(
                StatusCode::FORBIDDEN,
                "comment_blocked",
                message,
            ));
        }
        CommentDecision::Allow {
            author_name,
            email,
            content,
        } => (author_name, email, content),
    };
    // 插件改写后的字段仍需满足基本约束
    if author_name.trim().is_empty() {
        return Err(ApiError::validation("author_name 不能为空"));
    }
    if content.trim().is_empty() {
        return Err(ApiError::validation("content 不能为空"));
    }

    let created_at = now_rfc3339();
    let mut conn = pool.acquire().await?;
    sqlx::query(
        "INSERT INTO comments (post_id, target_type, author_name, email, content, status, \
         created_at) VALUES (?, 'page', ?, ?, ?, 'approved', ?)",
    )
    .bind(page_id)
    .bind(author_name.trim())
    .bind(email.as_deref().map(str::trim).filter(|e| !e.is_empty()))
    .bind(content.trim())
    .bind(&created_at)
    .execute(&mut *conn)
    .await?;
    let id = last_insert_id_on(&mut conn, &db_type).await?;

    Ok((
        StatusCode::CREATED,
        Json(CommentPub {
            id,
            author_name: author_name.trim().to_string(),
            content: content.trim().to_string(),
            created_at,
        }),
    ))
}
