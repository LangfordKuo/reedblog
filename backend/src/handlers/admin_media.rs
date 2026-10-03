//! 管理接口：媒体库（契约「媒体库」条款，2026-10-04 新增；全部需要 Bearer）
//! - GET /api/admin/media?page&per_page → 分页 [MediaItem]，created_at DESC, id DESC；
//!   列表请求时惰性扫描 uploads/ 补建历史文件记录（扫描失败只记日志，不阻断列表）
//! - DELETE /api/admin/media/:id → 204；同时删磁盘文件（缺失幂等）；
//!   路径词法清洗 + canonicalize 双防穿越，绝不删除 uploads 目录外文件

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use sqlx::Row;

use crate::error::{ApiError, ApiResult};
use crate::handlers::uploads::sanitize_upload_rel;
use crate::media;
use crate::models::{normalize_paging, MediaItem, MediaQuery, Page};
use crate::state::{require_pool, AppState};

use super::helpers::check_auth;

fn row_to_media_item(r: &sqlx::any::AnyRow) -> MediaItem {
    MediaItem {
        id: r.get::<i64, _>("id"),
        url: r.get::<String, _>("url"),
        filename: r.get::<String, _>("filename"),
        size: r.get::<i64, _>("size"),
        mime: r.get::<String, _>("mime"),
        width: r.try_get::<Option<i64>, _>("width").unwrap_or(None),
        height: r.try_get::<Option<i64>, _>("height").unwrap_or(None),
        created_at: r.get::<String, _>("created_at"),
    }
}

/// GET /api/admin/media?page&per_page → 分页 [MediaItem]，created_at DESC, id DESC
pub async fn admin_list_media(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<MediaQuery>,
) -> ApiResult<Json<Page<MediaItem>>> {
    check_auth(&state, &headers).await?;
    let (pool, db_type) = require_pool(&state).await?;

    // 历史文件兜底（惰性扫描）：失败只记日志，列表接口照常返回（契约「媒体库」条款）
    if let Err(e) = media::scan_uploads_into_media(&state, &pool, &db_type).await {
        eprintln!("[reedblog] warning: uploads 目录扫描失败（媒体库列表继续）: {e}");
    }

    let (page, per_page) = normalize_paging(q.page, q.per_page);
    let total: i64 = sqlx::query("SELECT COUNT(*) FROM media")
        .fetch_one(&pool)
        .await?
        .get(0);

    // created_at DESC（秒精度，字典序即时间序）；同秒行以 id DESC 兜底保证确定性
    let sql = format!(
        "SELECT {} FROM media ORDER BY created_at DESC, id DESC LIMIT ? OFFSET ?",
        media::MEDIA_COLUMNS
    );
    let rows = sqlx::query(&sql)
        .bind(per_page)
        .bind((page - 1) * per_page)
        .fetch_all(&pool)
        .await?;

    Ok(Json(Page {
        items: rows.iter().map(row_to_media_item).collect(),
        total,
        page,
        per_page,
    }))
}

/// 删除 media 记录对应的磁盘文件（词法清洗 + canonicalize 双防穿越，绝不出 uploads 根目录）。
/// 文件不存在视为已删除（幂等）；路径非法/解析越界只记日志、跳过删文件（记录照常删除）；
/// 其余 IO 失败上抛 500（调用方保留记录，避免文件残留被惰性扫描复活）。
fn remove_media_file(state: &AppState, url: &str) -> ApiResult<()> {
    let Some(rel) = url.strip_prefix("/api/uploads/") else {
        eprintln!("[reedblog] warning: media url 前缀异常，跳过删文件: {url}");
        return Ok(());
    };
    let Some(full) = sanitize_upload_rel(rel) else {
        eprintln!("[reedblog] warning: media url 路径非法，跳过删文件: {url}");
        return Ok(());
    };

    let root = state.uploads_dir().to_path_buf();
    let target = root.join(&full);
    let (Ok(root_canon), Ok(target_canon)) = (root.canonicalize(), target.canonicalize()) else {
        // 文件（或 uploads 根）不存在 → 已无可删，幂等成功
        return Ok(());
    };
    if !target_canon.starts_with(&root_canon) {
        eprintln!("[reedblog] warning: media url 解析越出 uploads 根目录，跳过删文件: {url}");
        return Ok(());
    }
    match std::fs::remove_file(&target_canon) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(ApiError::internal(format!("删除文件失败: {e}"))),
    }
}

/// DELETE /api/admin/media/:id → 204；先删文件再删记录（不留孤儿、不被惰性扫描复活）
pub async fn admin_delete_media(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<i64>,
) -> ApiResult<StatusCode> {
    check_auth(&state, &headers).await?;
    let (pool, _db_type) = require_pool(&state).await?;

    let row = sqlx::query("SELECT url FROM media WHERE id = ?")
        .bind(id)
        .fetch_optional(&pool)
        .await?
        .ok_or_else(ApiError::not_found)?;
    let url: String = row.get("url");

    remove_media_file(&state, &url)?;

    let deleted = sqlx::query("DELETE FROM media WHERE id = ?")
        .bind(id)
        .execute(&pool)
        .await?
        .rows_affected();
    if deleted == 0 {
        return Err(ApiError::not_found());
    }
    Ok(StatusCode::NO_CONTENT)
}
