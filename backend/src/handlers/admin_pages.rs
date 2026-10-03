//! 管理接口：页面管理（契约「页面」条款，全部需要 Bearer）
//! - GET    /api/admin/pages           → [PageAdmin]（含停用页，sort_order ASC, id ASC）
//! - POST   /api/admin/pages           → 201 PageAdmin（创建自定义页面，kind 恒为 custom）
//! - GET    /api/admin/pages/:id       → PageAdmin
//! - PUT    /api/admin/pages/:id       → PageAdmin（kind 不可改；links 全量替换，仅 kind=links 生效）
//! - PATCH  /api/admin/pages/:id/toggle → PageAdmin（enabled 取反）
//! - DELETE /api/admin/pages/:id       → 204（built_in → 422 page_builtin）

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use sqlx::AnyPool;
use sqlx::Row;

use crate::error::{ApiError, ApiResult, ValidJson};
use crate::models::{PageAdmin, PageBody};
use crate::pages::{
    fetch_page_links, replace_page_links, row_bool, row_to_page_admin, KIND_CUSTOM, KIND_LINKS,
    PAGE_COLUMNS,
};
use crate::state::{now_rfc3339, require_pool, AppState};

use super::helpers::{
    check_auth, is_unique_violation, last_insert_id_on, render_markdown, slugify, temp_slug,
};

fn slug_conflict(slug: &str) -> ApiError {
    ApiError::conflict("slug_taken", format!("slug '{slug}' 已被占用"))
}

/// 页面 slug 是否已被占用（pages 表内唯一；与文章 slug 各自独立命名空间）
async fn page_slug_taken(pool: &AnyPool, slug: &str, exclude_id: Option<i64>) -> ApiResult<bool> {
    let (sql, count) = match exclude_id {
        Some(id) => (
            "SELECT COUNT(*) FROM pages WHERE slug = ? AND id <> ?",
            Some(id),
        ),
        None => ("SELECT COUNT(*) FROM pages WHERE slug = ?", None),
    };
    let mut q = sqlx::query(sql).bind(slug);
    if let Some(id) = count {
        q = q.bind(id);
    }
    let n: i64 = q.fetch_one(pool).await?.get(0);
    Ok(n > 0)
}

async fn load_page_row(pool: &AnyPool, id: i64) -> ApiResult<Option<sqlx::any::AnyRow>> {
    let sql = format!("SELECT {PAGE_COLUMNS} FROM pages WHERE id = ?");
    Ok(sqlx::query(&sql).bind(id).fetch_optional(pool).await?)
}

async fn load_page_admin(pool: &AnyPool, id: i64) -> ApiResult<Option<PageAdmin>> {
    match load_page_row(pool, id).await? {
        None => Ok(None),
        Some(r) => {
            let links = fetch_page_links(pool, id).await?;
            Ok(Some(row_to_page_admin(&r, links)))
        }
    }
}

fn validate_title(t: &str) -> ApiResult<String> {
    let t = t.trim().to_string();
    if t.is_empty() {
        return Err(ApiError::validation("title 必填且不能为空"));
    }
    if t.chars().count() > 255 {
        return Err(ApiError::validation("title 不能超过 255 字符"));
    }
    Ok(t)
}

/// GET /api/admin/pages → [PageAdmin]（含停用页；页面数量少不分页）
pub async fn admin_list_pages(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> ApiResult<Json<Vec<PageAdmin>>> {
    check_auth(&state, &headers).await?;
    let (pool, _db_type) = require_pool(&state).await?;
    let sql = format!("SELECT {PAGE_COLUMNS} FROM pages ORDER BY sort_order ASC, id ASC");
    let rows = sqlx::query(&sql).fetch_all(&pool).await?;
    let mut items = Vec::with_capacity(rows.len());
    for r in &rows {
        let links = fetch_page_links(&pool, r.get::<i64, _>("id")).await?;
        items.push(row_to_page_admin(r, links));
    }
    Ok(Json(items))
}

/// GET /api/admin/pages/:id → PageAdmin
pub async fn admin_get_page(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<i64>,
) -> ApiResult<Json<PageAdmin>> {
    check_auth(&state, &headers).await?;
    let (pool, _db_type) = require_pool(&state).await?;
    load_page_admin(&pool, id)
        .await?
        .map(Json)
        .ok_or_else(ApiError::not_found)
}

/// POST /api/admin/pages → 201 PageAdmin（创建自定义页面，kind 恒为 custom，built_in=0）
pub async fn admin_create_page(
    State(state): State<AppState>,
    headers: HeaderMap,
    req_body: ValidJson<PageBody>,
) -> ApiResult<(StatusCode, Json<PageAdmin>)> {
    check_auth(&state, &headers).await?;
    let Json(body) = req_body.map_err(ApiError::from)?;
    let (pool, db_type) = require_pool(&state).await?;

    let title = validate_title(&body.title.unwrap_or_default())?;
    let content_md = body
        .content_md
        .ok_or_else(|| ApiError::validation("content_md 必填"))?;
    if content_md.chars().count() > 1_000_000 {
        return Err(ApiError::validation("content_md 过长"));
    }
    let enabled = body.enabled.unwrap_or(true);
    let sort_order = body.sort_order.unwrap_or(0);

    // slug：提供则原样使用；为空自动生成（ASCII slugify；纯中文回退 page-<id>，插入后回填）
    let provided_slug = body
        .slug
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let (mut slug, need_backfill) = match provided_slug {
        Some(s) => {
            if s.chars().count() > 255 {
                return Err(ApiError::validation("slug 不能超过 255 字符"));
            }
            if page_slug_taken(&pool, &s, None).await? {
                return Err(slug_conflict(&s));
            }
            (s, false)
        }
        None => {
            let gen = slugify(&title);
            if gen.is_empty() {
                (temp_slug(), true)
            } else {
                if page_slug_taken(&pool, &gen, None).await? {
                    return Err(slug_conflict(&gen));
                }
                (gen, false)
            }
        }
    };

    let now = now_rfc3339();
    let content_html = render_markdown(&content_md);
    let enabled_flag: i64 = if enabled { 1 } else { 0 };

    let mut conn = pool.acquire().await?;
    let insert = sqlx::query(
        "INSERT INTO pages (title, slug, kind, content_md, content_html, enabled, sort_order, \
         built_in, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?, 0, ?, ?)",
    )
    .bind(&title)
    .bind(&slug)
    .bind(KIND_CUSTOM)
    .bind(&content_md)
    .bind(&content_html)
    .bind(enabled_flag)
    .bind(sort_order)
    .bind(&now)
    .bind(&now);
    if let Err(e) = insert.execute(&mut *conn).await {
        if is_unique_violation(&e) {
            return Err(slug_conflict(&slug));
        }
        return Err(e.into());
    }
    let id = last_insert_id_on(&mut conn, &db_type).await?;
    if need_backfill {
        slug = format!("page-{id}");
        sqlx::query("UPDATE pages SET slug = ? WHERE id = ?")
            .bind(&slug)
            .bind(id)
            .execute(&mut *conn)
            .await?;
    }
    drop(conn);

    // 自定义页 kind=custom：links 字段忽略（契约「页面」条款）
    let page = load_page_admin(&pool, id)
        .await?
        .ok_or_else(|| ApiError::internal("创建后读取页面失败"))?;
    Ok((StatusCode::CREATED, Json(page)))
}

/// PUT /api/admin/pages/:id → PageAdmin（字段可选更新；kind 不可改；
/// links 全量替换语义，仅 kind=links 页面接受，其余 kind 忽略）
pub async fn admin_update_page(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<i64>,
    req_body: ValidJson<PageBody>,
) -> ApiResult<Json<PageAdmin>> {
    check_auth(&state, &headers).await?;
    let Json(body) = req_body.map_err(ApiError::from)?;
    let (pool, db_type) = require_pool(&state).await?;
    let existing = load_page_row(&pool, id)
        .await?
        .ok_or_else(ApiError::not_found)?;
    let kind = existing.get::<String, _>("kind");

    let title = match body.title {
        Some(t) => validate_title(&t)?,
        None => existing.get::<String, _>("title"),
    };
    let content_md = body
        .content_md
        .unwrap_or_else(|| existing.get::<String, _>("content_md"));
    let enabled = body
        .enabled
        .unwrap_or_else(|| row_bool(&existing, "enabled"));
    let sort_order = body
        .sort_order
        .unwrap_or_else(|| existing.get::<i64, _>("sort_order"));

    let mut slug = existing.get::<String, _>("slug");
    if let Some(s) = body
        .slug
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
    {
        if s.chars().count() > 255 {
            return Err(ApiError::validation("slug 不能超过 255 字符"));
        }
        if s != slug {
            if page_slug_taken(&pool, &s, Some(id)).await? {
                return Err(slug_conflict(&s));
            }
            slug = s;
        }
    }

    // links 先校验替换（仅 kind=links 生效），再更新页面本体
    if let Some(links) = &body.links {
        if kind == KIND_LINKS {
            replace_page_links(&pool, &db_type, id, links).await?;
        }
    }

    let now = now_rfc3339();
    let content_html = render_markdown(&content_md);
    let enabled_flag: i64 = if enabled { 1 } else { 0 };
    let update = sqlx::query(
        "UPDATE pages SET title = ?, slug = ?, content_md = ?, content_html = ?, enabled = ?, \
         sort_order = ?, updated_at = ? WHERE id = ?",
    )
    .bind(&title)
    .bind(&slug)
    .bind(&content_md)
    .bind(&content_html)
    .bind(enabled_flag)
    .bind(sort_order)
    .bind(&now)
    .bind(id);
    if let Err(e) = update.execute(&pool).await {
        if is_unique_violation(&e) {
            return Err(slug_conflict(&slug));
        }
        return Err(e.into());
    }

    let page = load_page_admin(&pool, id)
        .await?
        .ok_or_else(|| ApiError::internal("更新后读取页面失败"))?;
    Ok(Json(page))
}

/// PATCH /api/admin/pages/:id/toggle → PageAdmin（enabled 取反；
/// 停用后前台详情立即 404、公开列表/导航消失）
pub async fn admin_toggle_page(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<i64>,
) -> ApiResult<Json<PageAdmin>> {
    check_auth(&state, &headers).await?;
    let (pool, _db_type) = require_pool(&state).await?;
    let existing = load_page_row(&pool, id)
        .await?
        .ok_or_else(ApiError::not_found)?;

    let now = now_rfc3339();
    let enabled: i64 = if row_bool(&existing, "enabled") { 0 } else { 1 };
    sqlx::query("UPDATE pages SET enabled = ?, updated_at = ? WHERE id = ?")
        .bind(enabled)
        .bind(&now)
        .bind(id)
        .execute(&pool)
        .await?;

    let page = load_page_admin(&pool, id)
        .await?
        .ok_or_else(|| ApiError::internal("切换后读取页面失败"))?;
    Ok(Json(page))
}

/// DELETE /api/admin/pages/:id → 204（连带删除该页友情链接与页面留言）；
/// built_in → 422 page_builtin（内置页可停用、可编辑，但不可删除）
pub async fn admin_delete_page(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<i64>,
) -> ApiResult<StatusCode> {
    check_auth(&state, &headers).await?;
    let (pool, _db_type) = require_pool(&state).await?;
    let existing = load_page_row(&pool, id)
        .await?
        .ok_or_else(ApiError::not_found)?;
    if row_bool(&existing, "built_in") {
        return Err(ApiError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "page_builtin",
            "内置页面不可删除，可将其停用以在前台隐藏",
        ));
    }

    sqlx::query("DELETE FROM page_links WHERE page_id = ?")
        .bind(id)
        .execute(&pool)
        .await?;
    sqlx::query("DELETE FROM comments WHERE target_type = 'page' AND post_id = ?")
        .bind(id)
        .execute(&pool)
        .await?;
    sqlx::query("DELETE FROM pages WHERE id = ?")
        .bind(id)
        .execute(&pool)
        .await?;

    Ok(StatusCode::NO_CONTENT)
}
