//! 管理接口：分类/标签 CRUD（全部需要 Bearer）
//! 名称唯一 → 409 duplicate_name；删除时被文章引用 → 409 in_use。

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use sqlx::Row;

use crate::error::{ApiError, ApiResult, ValidJson};
use crate::models::{Category, NameBody, Tag};
use crate::state::{require_pool, AppState};

use super::helpers::{check_auth, is_unique_violation, last_insert_id_on};

fn duplicate_name(name: &str) -> ApiError {
    ApiError::conflict("duplicate_name", format!("名称 '{name}' 已存在"))
}

// ---------- 分类 ----------

/// GET /api/admin/categories → [Category]（post_count 只统计 published）
pub async fn admin_list_categories(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> ApiResult<Json<Vec<Category>>> {
    check_auth(&state, &headers).await?;
    let (pool, _db_type) = require_pool(&state).await?;
    let rows = sqlx::query(
        "SELECT c.id, c.name, COUNT(p.id) AS post_count FROM categories c \
         LEFT JOIN posts p ON p.category_id = c.id AND p.status = 'published' \
         GROUP BY c.id, c.name ORDER BY c.name",
    )
    .fetch_all(&pool)
    .await?;
    Ok(Json(
        rows.iter()
            .map(|r| Category {
                id: r.get::<i64, _>("id"),
                name: r.get::<String, _>("name"),
                post_count: r.get::<i64, _>("post_count"),
            })
            .collect(),
    ))
}

/// POST /api/admin/categories body {name} → 201 Category
pub async fn admin_create_category(
    State(state): State<AppState>,
    headers: HeaderMap,
    req_body: ValidJson<NameBody>,
) -> ApiResult<(StatusCode, Json<Category>)> {
    check_auth(&state, &headers).await?;
    let Json(body) = req_body.map_err(ApiError::from)?;
    let (pool, db_type) = require_pool(&state).await?;
    let name = body.name.trim().to_string();
    if name.is_empty() {
        return Err(ApiError::validation("name 不能为空"));
    }

    let mut conn = pool.acquire().await?;
    if let Err(e) = sqlx::query("INSERT INTO categories (name) VALUES (?)")
        .bind(&name)
        .execute(&mut *conn)
        .await
    {
        if is_unique_violation(&e) {
            return Err(duplicate_name(&name));
        }
        return Err(e.into());
    }
    let id = last_insert_id_on(&mut conn, &db_type).await?;
    Ok((
        StatusCode::CREATED,
        Json(Category {
            id,
            name,
            post_count: 0,
        }),
    ))
}

/// PUT /api/admin/categories/:id body {name} → Category
pub async fn admin_update_category(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<i64>,
    req_body: ValidJson<NameBody>,
) -> ApiResult<Json<Category>> {
    check_auth(&state, &headers).await?;
    let Json(body) = req_body.map_err(ApiError::from)?;
    let (pool, _db_type) = require_pool(&state).await?;
    let name = body.name.trim().to_string();
    if name.is_empty() {
        return Err(ApiError::validation("name 不能为空"));
    }

    let count: i64 = sqlx::query("SELECT COUNT(*) FROM categories WHERE id = ?")
        .bind(id)
        .fetch_one(&pool)
        .await?
        .get(0);
    if count == 0 {
        return Err(ApiError::not_found());
    }

    if let Err(e) = sqlx::query("UPDATE categories SET name = ? WHERE id = ?")
        .bind(&name)
        .bind(id)
        .execute(&pool)
        .await
    {
        if is_unique_violation(&e) {
            return Err(duplicate_name(&name));
        }
        return Err(e.into());
    }

    let post_count: i64 =
        sqlx::query("SELECT COUNT(*) FROM posts WHERE category_id = ? AND status = 'published'")
            .bind(id)
            .fetch_one(&pool)
            .await?
            .get(0);
    Ok(Json(Category {
        id,
        name,
        post_count,
    }))
}

/// DELETE /api/admin/categories/:id → 204；分类下有文章 → 409 in_use
pub async fn admin_delete_category(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<i64>,
) -> ApiResult<StatusCode> {
    check_auth(&state, &headers).await?;
    let (pool, _db_type) = require_pool(&state).await?;

    let count: i64 = sqlx::query("SELECT COUNT(*) FROM categories WHERE id = ?")
        .bind(id)
        .fetch_one(&pool)
        .await?
        .get(0);
    if count == 0 {
        return Err(ApiError::not_found());
    }

    let in_use: i64 = sqlx::query("SELECT COUNT(*) FROM posts WHERE category_id = ?")
        .bind(id)
        .fetch_one(&pool)
        .await?
        .get(0);
    if in_use > 0 {
        return Err(ApiError::conflict("in_use", "该分类下仍有文章，不能删除"));
    }

    sqlx::query("DELETE FROM categories WHERE id = ?")
        .bind(id)
        .execute(&pool)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

// ---------- 标签 ----------

/// GET /api/admin/tags → [Tag]（post_count 只统计 published）
pub async fn admin_list_tags(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> ApiResult<Json<Vec<Tag>>> {
    check_auth(&state, &headers).await?;
    let (pool, _db_type) = require_pool(&state).await?;
    let rows = sqlx::query(
        "SELECT t.id, t.name, COUNT(p.id) AS post_count FROM tags t \
         LEFT JOIN post_tags pt ON pt.tag_id = t.id \
         LEFT JOIN posts p ON p.id = pt.post_id AND p.status = 'published' \
         GROUP BY t.id, t.name ORDER BY t.name",
    )
    .fetch_all(&pool)
    .await?;
    Ok(Json(
        rows.iter()
            .map(|r| Tag {
                id: r.get::<i64, _>("id"),
                name: r.get::<String, _>("name"),
                post_count: r.get::<i64, _>("post_count"),
            })
            .collect(),
    ))
}

/// POST /api/admin/tags body {name} → 201 Tag
pub async fn admin_create_tag(
    State(state): State<AppState>,
    headers: HeaderMap,
    req_body: ValidJson<NameBody>,
) -> ApiResult<(StatusCode, Json<Tag>)> {
    check_auth(&state, &headers).await?;
    let Json(body) = req_body.map_err(ApiError::from)?;
    let (pool, db_type) = require_pool(&state).await?;
    let name = body.name.trim().to_string();
    if name.is_empty() {
        return Err(ApiError::validation("name 不能为空"));
    }

    let mut conn = pool.acquire().await?;
    if let Err(e) = sqlx::query("INSERT INTO tags (name) VALUES (?)")
        .bind(&name)
        .execute(&mut *conn)
        .await
    {
        if is_unique_violation(&e) {
            return Err(duplicate_name(&name));
        }
        return Err(e.into());
    }
    let id = last_insert_id_on(&mut conn, &db_type).await?;
    Ok((
        StatusCode::CREATED,
        Json(Tag {
            id,
            name,
            post_count: 0,
        }),
    ))
}

/// PUT /api/admin/tags/:id body {name} → Tag
pub async fn admin_update_tag(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<i64>,
    req_body: ValidJson<NameBody>,
) -> ApiResult<Json<Tag>> {
    check_auth(&state, &headers).await?;
    let Json(body) = req_body.map_err(ApiError::from)?;
    let (pool, _db_type) = require_pool(&state).await?;
    let name = body.name.trim().to_string();
    if name.is_empty() {
        return Err(ApiError::validation("name 不能为空"));
    }

    let count: i64 = sqlx::query("SELECT COUNT(*) FROM tags WHERE id = ?")
        .bind(id)
        .fetch_one(&pool)
        .await?
        .get(0);
    if count == 0 {
        return Err(ApiError::not_found());
    }

    if let Err(e) = sqlx::query("UPDATE tags SET name = ? WHERE id = ?")
        .bind(&name)
        .bind(id)
        .execute(&pool)
        .await
    {
        if is_unique_violation(&e) {
            return Err(duplicate_name(&name));
        }
        return Err(e.into());
    }

    let post_count: i64 = sqlx::query(
        "SELECT COUNT(*) FROM post_tags pt JOIN posts p ON p.id = pt.post_id \
         WHERE pt.tag_id = ? AND p.status = 'published'",
    )
    .bind(id)
    .fetch_one(&pool)
    .await?
    .get(0);
    Ok(Json(Tag {
        id,
        name,
        post_count,
    }))
}

/// DELETE /api/admin/tags/:id → 204；标签被文章引用 → 409 in_use
pub async fn admin_delete_tag(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<i64>,
) -> ApiResult<StatusCode> {
    check_auth(&state, &headers).await?;
    let (pool, _db_type) = require_pool(&state).await?;

    let count: i64 = sqlx::query("SELECT COUNT(*) FROM tags WHERE id = ?")
        .bind(id)
        .fetch_one(&pool)
        .await?
        .get(0);
    if count == 0 {
        return Err(ApiError::not_found());
    }

    let in_use: i64 = sqlx::query("SELECT COUNT(*) FROM post_tags WHERE tag_id = ?")
        .bind(id)
        .fetch_one(&pool)
        .await?
        .get(0);
    if in_use > 0 {
        return Err(ApiError::conflict("in_use", "该标签仍被文章引用，不能删除"));
    }

    sqlx::query("DELETE FROM tags WHERE id = ?")
        .bind(id)
        .execute(&pool)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}
