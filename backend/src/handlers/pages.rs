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
use crate::state::{require_pool, AppState};

use super::helpers::{
    create_comment_pipeline, render_markdown, row_to_comment_pub, PUBLIC_COMMENT_COLUMNS,
    PUBLIC_COMMENT_FROM, PUBLIC_COMMENT_THREAD_FILTER,
};

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

/// GET /api/pages/:slug/comments → [CommentPub]（仅 approved 且线程可见，时间 ASC, id ASC）
///
/// 与文章评论同款形状与线程规则（契约「评论回复」条款）：平铺数组带
/// parent_id/reply_to_id/reply_to_name，hidden 顶级留言的整条线程不出现。
pub async fn list_page_comments(
    State(state): State<AppState>,
    Path(slug): Path<String>,
) -> ApiResult<Json<Vec<CommentPub>>> {
    let (pool, _db_type) = require_pool(&state).await?;
    let page_id = find_message_board_page(&pool, &slug).await?;
    let sql = format!(
        "SELECT {PUBLIC_COMMENT_COLUMNS} {PUBLIC_COMMENT_FROM} \
         WHERE c.target_type = 'page' AND c.post_id = ? AND c.status = 'approved' \
         AND {PUBLIC_COMMENT_THREAD_FILTER} \
         ORDER BY c.created_at ASC, c.id ASC"
    );
    let rows = sqlx::query(&sql).bind(page_id).fetch_all(&pool).await?;
    Ok(Json(rows.iter().map(row_to_comment_pub).collect()))
}

/// POST /api/pages/:slug/comments → 201 CommentPub（先发后审：创建即 approved）
///
/// 与文章评论同一条创建管线（helpers::create_comment_pipeline）：必填校验 →
/// 父留言校验与两级归一化（body 可选 parent_id）→ comment.before_create 钩子链
/// （block → 403 comment_blocked；ctx.post_slug = 页面 slug）→ 插入 target_type='page' 的评论行。
pub async fn create_page_comment(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    body: ValidJson<CreateCommentRequest>,
) -> ApiResult<(StatusCode, Json<CommentPub>)> {
    let Json(req) = body.map_err(ApiError::from)?;
    let (pool, db_type) = require_pool(&state).await?;
    let page_id = find_message_board_page(&pool, &slug).await?;
    let created = create_comment_pipeline(
        &state,
        &pool,
        &db_type,
        "page",
        page_id,
        &slug,
        req,
        "留言被插件拦截",
    )
    .await?;
    Ok((StatusCode::CREATED, Json(created)))
}
