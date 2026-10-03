//! 管理接口：文章 CRUD（全部需要 Bearer）

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use sqlx::any::AnyRow;
use sqlx::AnyPool;
use sqlx::Row;

use crate::error::{ApiError, ApiResult, ValidJson};
use crate::models::{normalize_paging, AdminPostsQuery, Page, PostAdmin, PostBody};
use crate::state::{now_rfc3339, require_pool, AppState};

use super::helpers::{
    check_auth, derive_excerpt, ensure_category_exists, fetch_tag_ids, is_unique_violation,
    last_insert_id_on, replace_post_tags, slug_taken, slugify, temp_slug,
};

const ADMIN_COLUMNS: &str = "p.id, p.title, p.slug, p.content_md, p.excerpt, p.status, \
     p.category_id, c.name AS category_name, p.published_at, p.created_at, p.updated_at";

const ADMIN_FROM: &str = "FROM posts p LEFT JOIN categories c ON c.id = p.category_id";

async fn row_to_post_admin(pool: &AnyPool, r: &AnyRow) -> ApiResult<PostAdmin> {
    let id = r.get::<i64, _>("id");
    Ok(PostAdmin {
        id,
        title: r.get::<String, _>("title"),
        slug: r.get::<String, _>("slug"),
        content_md: r.get::<String, _>("content_md"),
        excerpt: r.get::<String, _>("excerpt"),
        status: r.get::<String, _>("status"),
        category_id: r.try_get::<Option<i64>, _>("category_id").unwrap_or(None),
        category_name: r
            .try_get::<Option<String>, _>("category_name")
            .unwrap_or(None),
        tag_ids: fetch_tag_ids(pool, id).await?,
        published_at: r
            .try_get::<Option<String>, _>("published_at")
            .unwrap_or(None),
        created_at: r.get::<String, _>("created_at"),
        updated_at: r.get::<String, _>("updated_at"),
    })
}

async fn load_post_admin(pool: &AnyPool, id: i64) -> ApiResult<Option<PostAdmin>> {
    let sql = format!("SELECT {ADMIN_COLUMNS} {ADMIN_FROM} WHERE p.id = ?");
    let row = sqlx::query(&sql).bind(id).fetch_optional(pool).await?;
    match row {
        None => Ok(None),
        Some(r) => Ok(Some(row_to_post_admin(pool, &r).await?)),
    }
}

fn validate_status(s: &str) -> ApiResult<()> {
    if s != "draft" && s != "published" {
        return Err(ApiError::validation("status 必须是 draft 或 published"));
    }
    Ok(())
}

fn slug_conflict(slug: &str) -> ApiError {
    ApiError::conflict("slug_taken", format!("slug '{slug}' 已被占用"))
}

/// GET /api/admin/posts?status=<draft|published|all>&page&per_page → 分页 [PostAdmin]，updated_at DESC
pub async fn admin_list_posts(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<AdminPostsQuery>,
) -> ApiResult<Json<Page<PostAdmin>>> {
    check_auth(&state, &headers).await?;
    let (pool, _db_type) = require_pool(&state).await?;
    let (page, per_page) = normalize_paging(q.page, q.per_page);

    let status = q.status.clone().unwrap_or_else(|| "all".to_string());
    let param: Option<String> = if status == "all" {
        None
    } else {
        validate_status(&status)?;
        Some(status)
    };
    let where_sql = if param.is_some() {
        "WHERE p.status = ?"
    } else {
        ""
    };

    let count_sql = format!("SELECT COUNT(*) FROM posts p {where_sql}");
    let mut cq = sqlx::query(&count_sql);
    if let Some(s) = &param {
        cq = cq.bind(s.clone());
    }
    let total: i64 = cq.fetch_one(&pool).await?.get(0);

    let list_sql = format!(
        "SELECT {ADMIN_COLUMNS} {ADMIN_FROM} {where_sql} \
         ORDER BY p.updated_at DESC LIMIT ? OFFSET ?"
    );
    let mut lq = sqlx::query(&list_sql);
    if let Some(s) = &param {
        lq = lq.bind(s.clone());
    }
    let rows = lq
        .bind(per_page)
        .bind((page - 1) * per_page)
        .fetch_all(&pool)
        .await?;

    let mut items = Vec::with_capacity(rows.len());
    for r in &rows {
        items.push(row_to_post_admin(&pool, r).await?);
    }
    Ok(Json(Page {
        items,
        total,
        page,
        per_page,
    }))
}

/// POST /api/admin/posts → 201 PostAdmin
pub async fn admin_create_post(
    State(state): State<AppState>,
    headers: HeaderMap,
    req_body: ValidJson<PostBody>,
) -> ApiResult<(StatusCode, Json<PostAdmin>)> {
    check_auth(&state, &headers).await?;
    let Json(body) = req_body.map_err(ApiError::from)?;
    let (pool, db_type) = require_pool(&state).await?;

    let title = body
        .title
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
        .ok_or_else(|| ApiError::validation("title 必填且不能为空"))?;
    let content_md = body
        .content_md
        .ok_or_else(|| ApiError::validation("content_md 必填"))?;
    let status = body
        .status
        .ok_or_else(|| ApiError::validation("status 必填"))?;
    validate_status(&status)?;

    let category_id = match body.category_id {
        Some(Some(cid)) => {
            ensure_category_exists(&pool, cid).await?;
            Some(cid)
        }
        _ => None,
    };

    let excerpt = body
        .excerpt
        .map(|e| e.trim().to_string())
        .filter(|e| !e.is_empty())
        .unwrap_or_else(|| derive_excerpt(&content_md));

    // slug：提供则原样使用；为空自动生成（ASCII slugify；纯中文回退 post-<id>，插入后回填）
    let provided_slug = body
        .slug
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let (mut slug, need_backfill) = match provided_slug {
        Some(s) => {
            if slug_taken(&pool, &s, None).await? {
                return Err(slug_conflict(&s));
            }
            (s, false)
        }
        None => {
            let gen = slugify(&title);
            if gen.is_empty() {
                (temp_slug(), true)
            } else {
                if slug_taken(&pool, &gen, None).await? {
                    return Err(slug_conflict(&gen));
                }
                (gen, false)
            }
        }
    };

    let now = now_rfc3339();
    // status=published 首次发布 → 写 published_at
    let published_at = if status == "published" {
        Some(now.clone())
    } else {
        None
    };

    let mut conn = pool.acquire().await?;
    let insert = sqlx::query(
        "INSERT INTO posts (title, slug, excerpt, content_md, status, category_id, \
         published_at, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&title)
    .bind(&slug)
    .bind(&excerpt)
    .bind(&content_md)
    .bind(&status)
    .bind(category_id)
    .bind(published_at.as_deref())
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
        slug = format!("post-{id}");
        sqlx::query("UPDATE posts SET slug = ? WHERE id = ?")
            .bind(&slug)
            .bind(id)
            .execute(&mut *conn)
            .await?;
    }
    drop(conn);

    if let Some(tag_ids) = &body.tag_ids {
        replace_post_tags(&pool, id, tag_ids).await?;
    }

    let post = load_post_admin(&pool, id)
        .await?
        .ok_or_else(|| ApiError::internal("创建后读取文章失败"))?;

    // post.after_publish 通知钩子（扩展契约）：创建即发布时触发；返回值忽略、错误不阻断
    if status == "published" {
        if let Some(pa) = post.published_at.as_deref() {
            state
                .plugins()
                .run_post_after_publish(&post.title, &post.slug, pa)
                .await;
        }
    }

    Ok((StatusCode::CREATED, Json(post)))
}

/// GET /api/admin/posts/:id → PostAdmin
pub async fn admin_get_post(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<i64>,
) -> ApiResult<Json<PostAdmin>> {
    check_auth(&state, &headers).await?;
    let (pool, _db_type) = require_pool(&state).await?;
    load_post_admin(&pool, id)
        .await?
        .map(Json)
        .ok_or_else(ApiError::not_found)
}

/// PUT /api/admin/posts/:id → PostAdmin（字段可选更新；draft→published 补写 published_at）
pub async fn admin_update_post(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<i64>,
    req_body: ValidJson<PostBody>,
) -> ApiResult<Json<PostAdmin>> {
    check_auth(&state, &headers).await?;
    let Json(body) = req_body.map_err(ApiError::from)?;
    let (pool, _db_type) = require_pool(&state).await?;
    let existing = load_post_admin(&pool, id)
        .await?
        .ok_or_else(ApiError::not_found)?;

    let title = match body.title {
        Some(t) => {
            let t = t.trim().to_string();
            if t.is_empty() {
                return Err(ApiError::validation("title 不能为空"));
            }
            t
        }
        None => existing.title.clone(),
    };
    let content_md = body.content_md.unwrap_or(existing.content_md.clone());
    let excerpt = match body.excerpt {
        Some(e) => e.trim().to_string(),
        None => existing.excerpt.clone(),
    };
    let status = match body.status {
        Some(s) => {
            validate_status(&s)?;
            s
        }
        None => existing.status.clone(),
    };
    let category_id = match body.category_id {
        Some(Some(cid)) => {
            ensure_category_exists(&pool, cid).await?;
            Some(cid)
        }
        Some(None) => None, // 显式 null → 清空分类
        None => existing.category_id,
    };

    let mut slug = existing.slug.clone();
    if let Some(s) = body
        .slug
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
    {
        if s != slug {
            if slug_taken(&pool, &s, Some(id)).await? {
                return Err(slug_conflict(&s));
            }
            slug = s;
        }
    }

    let now = now_rfc3339();
    // draft→published 时若 published_at 为空则写入；其余保持原值
    let newly_published = status == "published" && existing.published_at.is_none();
    let published_at = if newly_published {
        Some(now.clone())
    } else {
        existing.published_at.clone()
    };

    let update = sqlx::query(
        "UPDATE posts SET title = ?, slug = ?, excerpt = ?, content_md = ?, status = ?, \
         category_id = ?, published_at = ?, updated_at = ? WHERE id = ?",
    )
    .bind(&title)
    .bind(&slug)
    .bind(&excerpt)
    .bind(&content_md)
    .bind(&status)
    .bind(category_id)
    .bind(published_at.as_deref())
    .bind(&now)
    .bind(id);
    if let Err(e) = update.execute(&pool).await {
        if is_unique_violation(&e) {
            return Err(slug_conflict(&slug));
        }
        return Err(e.into());
    }

    if let Some(tag_ids) = &body.tag_ids {
        replace_post_tags(&pool, id, tag_ids).await?;
    }

    let post = load_post_admin(&pool, id)
        .await?
        .ok_or_else(|| ApiError::internal("更新后读取文章失败"))?;

    // post.after_publish 通知钩子（扩展契约）：draft→published 首次发布时触发
    if newly_published {
        if let Some(pa) = post.published_at.as_deref() {
            state
                .plugins()
                .run_post_after_publish(&post.title, &post.slug, pa)
                .await;
        }
    }

    Ok(Json(post))
}

/// DELETE /api/admin/posts/:id → 204（连带清理标签关联与评论）
pub async fn admin_delete_post(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<i64>,
) -> ApiResult<StatusCode> {
    check_auth(&state, &headers).await?;
    let (pool, _db_type) = require_pool(&state).await?;

    let count: i64 = sqlx::query("SELECT COUNT(*) FROM posts WHERE id = ?")
        .bind(id)
        .fetch_one(&pool)
        .await?
        .get(0);
    if count == 0 {
        return Err(ApiError::not_found());
    }

    sqlx::query("DELETE FROM post_tags WHERE post_id = ?")
        .bind(id)
        .execute(&pool)
        .await?;
    sqlx::query("DELETE FROM comments WHERE post_id = ?")
        .bind(id)
        .execute(&pool)
        .await?;
    sqlx::query("DELETE FROM posts WHERE id = ?")
        .bind(id)
        .execute(&pool)
        .await?;

    Ok(StatusCode::NO_CONTENT)
}
