//! 媒体库领域逻辑（契约「媒体库」条款，2026-10-04 新增）：
//! - media 表 upsert（以存储 url 唯一，重复上传同图返回既有记录）
//! - uploads 目录惰性扫描：把没有对应记录的历史文件补成记录（列表接口兜底）
//! 全部 SQL 只用 SQLite/MySQL 共用写法（先查后插 + 唯一约束冲突兜底），
//! 不用 ON CONFLICT / ON DUPLICATE KEY 等单方言语法。

use sqlx::AnyPool;
use sqlx::Row;

use crate::error::{ApiError, ApiResult};
use crate::handlers::helpers::is_unique_violation;
use crate::handlers::uploads::{detect_image_kind, image_dimensions};
use crate::state::{last_insert_id, now_rfc3339, AppState};

/// 列表查询列集（顺序与 row_to_media_item 对应）
pub const MEDIA_COLUMNS: &str = "id, url, filename, size, mime, width, height, created_at";

/// 文件名入库前按 char 截断（MySQL VARCHAR(255) 上限；不影响 UploadResult 回显）
fn clamp_filename(name: &str) -> String {
    name.chars().take(255).collect()
}

/// 按 url 查 media 记录 id（不存在 → None）
pub async fn find_media_id_by_url(pool: &AnyPool, url: &str) -> Result<Option<i64>, sqlx::Error> {
    let row = sqlx::query("SELECT id FROM media WHERE url = ?")
        .bind(url)
        .fetch_optional(pool)
        .await?;
    Ok(row.map(|r| r.get::<i64, _>("id")))
}

/// 插入 media 记录；并发下撞 url 唯一约束时改为读取既有 id（幂等 upsert 的兜底路径）
pub async fn upsert_media_url_at(
    pool: &AnyPool,
    db_type: &str,
    url: &str,
    filename: &str,
    size: i64,
    mime: &str,
    width: Option<i64>,
    height: Option<i64>,
    created_at: &str,
) -> Result<i64, sqlx::Error> {
    if let Some(id) = find_media_id_by_url(pool, url).await? {
        return Ok(id);
    }
    let mut conn = pool.acquire().await?;
    let result = sqlx::query(
        "INSERT INTO media (url, filename, size, mime, width, height, created_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(url)
    .bind(clamp_filename(filename))
    .bind(size)
    .bind(mime)
    .bind(width)
    .bind(height)
    .bind(created_at)
    .execute(&mut *conn)
    .await;
    match result {
        Ok(_) => last_insert_id(&mut conn, db_type).await,
        Err(e) if is_unique_violation(&e) => find_media_id_by_url(pool, url)
            .await?
            .ok_or_else(|| sqlx::Error::Protocol("media 记录并发写入冲突，且回查失败".into())),
        Err(e) => Err(e),
    }
}

/// 上传链路 upsert：已存在返回既有 id，不存在插入（created_at = 当前时间）
pub async fn upsert_media_url(
    pool: &AnyPool,
    db_type: &str,
    url: &str,
    filename: &str,
    size: i64,
    mime: &str,
    width: Option<i64>,
    height: Option<i64>,
) -> ApiResult<i64> {
    upsert_media_url_at(
        pool,
        db_type,
        url,
        filename,
        size,
        mime,
        width,
        height,
        &now_rfc3339(),
    )
    .await
    .map_err(ApiError::from)
}

/// 扩展名 → MIME（仅图片扩展名；magic bytes 解析不出时兜底）
fn upload_ext_mime(name: &str) -> Option<&'static str> {
    match name.rsplit('.').next()?.to_ascii_lowercase().as_str() {
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "gif" => Some("image/gif"),
        "webp" => Some("image/webp"),
        _ => None,
    }
}

/// 文件 mtime → RFC3339 UTC 文本（取不到由调用方回退当前时间）
fn system_time_rfc3339(t: std::time::SystemTime) -> String {
    let dt: chrono::DateTime<chrono::Utc> = t.into();
    dt.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

/// 目录名是否为 yyyy / mm 形状（只扫描应用自身的存储布局，忽略杂项目录）
fn is_year_dir(name: &str) -> bool {
    name.len() == 4 && name.bytes().all(|b| b.is_ascii_digit())
}

fn is_month_dir(name: &str) -> bool {
    name.len() == 2 && name.bytes().all(|b| b.is_ascii_digit())
}

/// 惰性扫描 uploads/<yyyy>/<mm>/ 下尚无 media 记录的文件并补建记录（历史文件兜底）：
/// - 原始文件名用存储文件名；created_at 取文件 mtime（取不到用当前时间）
/// - 宽高按 magic bytes 尽力解析（解析不出与扩展名兜底都失败时跳过该文件）
/// - 目录不存在/不可读 → Ok(())（无历史文件可扫，不算失败）
/// 单个文件读取失败跳过；DB 错误向上抛，由调用方记日志（不得让列表接口报错）。
pub async fn scan_uploads_into_media(
    state: &AppState,
    pool: &AnyPool,
    db_type: &str,
) -> Result<(), sqlx::Error> {
    let Ok(years) = std::fs::read_dir(state.uploads_dir()) else {
        return Ok(());
    };
    for year in years.flatten() {
        let year_path = year.path();
        if !year_path.is_dir() {
            continue;
        }
        let Some(year_name) = year_path.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        if !is_year_dir(year_name) {
            continue;
        }
        let Ok(months) = std::fs::read_dir(&year_path) else {
            continue;
        };
        for month in months.flatten() {
            let month_path = month.path();
            if !month_path.is_dir() {
                continue;
            }
            let Some(month_name) = month_path.file_name().and_then(|s| s.to_str()) else {
                continue;
            };
            if !is_month_dir(month_name) {
                continue;
            }
            let Ok(files) = std::fs::read_dir(&month_path) else {
                continue;
            };
            for file in files.flatten() {
                let Ok(file_type) = file.file_type() else {
                    continue;
                };
                if !file_type.is_file() {
                    continue;
                }
                let Some(name) = file.file_name().to_str().map(str::to_string) else {
                    continue;
                };
                let url = format!("/api/uploads/{year_name}/{month_name}/{name}");
                if find_media_id_by_url(pool, &url).await?.is_some() {
                    continue;
                }
                let Ok(meta) = file.metadata() else {
                    continue;
                };
                let Ok(bytes) = std::fs::read(file.path()) else {
                    continue;
                };
                // 类型：magic bytes 优先，其次已知图片扩展名；两者都不认则跳过
                let kind = detect_image_kind(&bytes);
                let mime = match kind {
                    Some(k) => k.mime().to_string(),
                    None => match upload_ext_mime(&name) {
                        Some(m) => m.to_string(),
                        None => continue,
                    },
                };
                let (width, height) = match kind.and_then(|k| image_dimensions(k, &bytes)) {
                    Some((w, h)) => (Some(w as i64), Some(h as i64)),
                    None => (None, None),
                };
                let created_at = meta
                    .modified()
                    .ok()
                    .map(system_time_rfc3339)
                    .unwrap_or_else(now_rfc3339);
                upsert_media_url_at(
                    pool,
                    db_type,
                    &url,
                    &name,
                    meta.len() as i64,
                    &mime,
                    width,
                    height,
                    &created_at,
                )
                .await?;
            }
        }
    }
    Ok(())
}
