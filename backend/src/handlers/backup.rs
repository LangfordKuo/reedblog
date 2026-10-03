//! 备份与恢复 HTTP 接口（契约「备份与恢复」条款，2026-10-04 新增；全部需要 Bearer）：
//! - GET  /api/admin/backup/export → zip 流（临时文件分块流出，读完/断开自动删临时文件）
//! - POST /api/admin/backup/import（multipart：file=zip、confirm=REPLACE）
//! - GET  /api/admin/backup/info   → 最近一次导出信息（内存态）

use std::io::SeekFrom;
use std::sync::Arc;

use axum::body::{Body, Bytes};
use axum::extract::multipart::{Field, MultipartRejection};
use axum::extract::{Multipart, State};
use axum::http::{header, HeaderMap, HeaderValue};
use axum::response::{IntoResponse, Response};
use axum::Json;
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};
use tokio_stream::StreamExt;

use crate::backup::{
    self, confirmation_required, payload_too_large, BackupImportResult, TempPath, MAX_IMPORT_BYTES,
};
use crate::error::{ApiError, ApiResult};
use crate::state::{require_pool, AppState};

use super::helpers::check_auth;

/// HTTP 响应分块大小（临时 zip 分块流出）
const STREAM_CHUNK: usize = 64 * 1024;

/// GET /api/admin/backup/export → 200 zip 流
pub async fn admin_export_backup(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> ApiResult<Response> {
    check_auth(&state, &headers).await?;
    let (pool, db_type) = require_pool(&state).await?;

    let export = backup::build_export(&state, &pool, &db_type).await?;
    state.set_backup_last_export(&export.exported_at, export.size);

    // 分块流出临时文件：读完 / 客户端断开都由 Arc 守卫 Drop 时删文件
    let file = tokio::fs::File::open(export.tmp.path()).await?;
    let file = Arc::new(tokio::sync::Mutex::new(file));
    let guard = Arc::new(export.tmp);
    let size = export.size;
    let chunks = size.div_ceil(STREAM_CHUNK as u64);
    let stream = tokio_stream::iter(0..chunks).then(move |i| {
        let file = Arc::clone(&file);
        let guard = Arc::clone(&guard);
        async move {
            let _keep = guard; // 该 future 存活期间保底持有守卫（流结束才真正删文件）
            let mut f = file.lock().await;
            f.seek(SeekFrom::Start(i * STREAM_CHUNK as u64)).await?;
            let mut buf = vec![0u8; STREAM_CHUNK];
            let n = f.read(&mut buf).await?;
            buf.truncate(n);
            Ok::<Bytes, std::io::Error>(Bytes::from(buf))
        }
    });

    let stamp = chrono::Utc::now().format("%Y%m%d%H%M%S");
    let disposition =
        HeaderValue::from_str(&format!("attachment; filename=\"reedblog-backup-{stamp}.zip\""))
            .map_err(|_| ApiError::internal("生成下载头失败"))?;
    let content_length = HeaderValue::from_str(&size.to_string())
        .map_err(|_| ApiError::internal("生成下载头失败"))?;

    Ok((
        [
            (
                header::CONTENT_TYPE,
                HeaderValue::from_static("application/zip"),
            ),
            (header::CONTENT_DISPOSITION, disposition),
            (header::CONTENT_LENGTH, content_length),
        ],
        Body::from_stream(stream),
    )
        .into_response())
}

/// POST /api/admin/backup/import（multipart file=zip + confirm=REPLACE）→ 200 BackupImportResult
pub async fn admin_import_backup(
    State(state): State<AppState>,
    headers: HeaderMap,
    multipart: Result<Multipart, MultipartRejection>,
) -> ApiResult<Json<BackupImportResult>> {
    check_auth(&state, &headers).await?;
    let (pool, _db_type) = require_pool(&state).await?;

    let mut mp = multipart.map_err(|_| {
        ApiError::validation("需要 multipart/form-data，字段 file = 备份 zip、confirm = REPLACE")
    })?;

    // 上传 zip 流式落盘（不整读进内存）；confirm 可能出现在 file 前或后，全部读完再判断
    let tmp = TempPath::new("import");
    let mut file_received = false;
    let mut confirm: Option<String> = None;
    while let Some(field) = mp.next_field().await.map_err(|e| {
        let msg = e.to_string();
        if msg.contains("length limit") || msg.contains("too large") {
            payload_too_large()
        } else {
            ApiError::validation(format!("multipart 解析失败: {e}"))
        }
    })? {
        match field.name() {
            Some("file") => {
                stream_field_to_file(field, tmp.path()).await?;
                file_received = true;
            }
            Some("confirm") => {
                let bytes = read_field_limited(field, 64).await?;
                confirm = Some(String::from_utf8_lossy(&bytes).trim().to_string());
            }
            _ => { /* 未知字段忽略 */ }
        }
    }
    if !file_received {
        return Err(ApiError::validation("缺少 file 字段（备份 zip）"));
    }
    if confirm.as_deref() != Some("REPLACE") {
        return Err(confirmation_required());
    }

    let result = backup::import_backup(&state, &pool, tmp.path()).await?;
    Ok(Json(result))
}

/// GET /api/admin/backup/info → 最近一次导出信息（内存态，重启清零）
pub async fn admin_backup_info(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> ApiResult<Json<backup::BackupInfo>> {
    check_auth(&state, &headers).await?;
    let last = state.backup_last_export();
    Ok(Json(backup::BackupInfo {
        last_export_at: last.as_ref().map(|(at, _)| at.clone()),
        total_size_bytes: last.map(|(_, size)| size),
    }))
}

/// multipart 文件字段流式落盘（逐块写，不整读进内存）；超上限 → 413
async fn stream_field_to_file(mut field: Field<'_>, path: &std::path::Path) -> ApiResult<u64> {
    let mut file = tokio::fs::File::create(path).await?;
    let mut total: usize = 0;
    loop {
        match field.chunk().await {
            Ok(Some(chunk)) => {
                total = total.saturating_add(chunk.len());
                if total > MAX_IMPORT_BYTES {
                    return Err(payload_too_large());
                }
                file.write_all(&chunk).await?;
            }
            Ok(None) => break,
            Err(e) => {
                let msg = e.to_string();
                if msg.contains("length limit") || msg.contains("too large") {
                    return Err(payload_too_large());
                }
                return Err(ApiError::validation(format!("读取上传内容失败: {e}")));
            }
        }
    }
    file.flush().await?;
    Ok(total as u64)
}

/// 读取 multipart 小文本字段（confirm），超长直接报 422
async fn read_field_limited(mut field: Field<'_>, max: usize) -> ApiResult<Vec<u8>> {
    let mut buf = Vec::new();
    loop {
        match field.chunk().await {
            Ok(Some(chunk)) => {
                if buf.len() + chunk.len() > max {
                    return Err(ApiError::validation("字段内容超长"));
                }
                buf.extend_from_slice(&chunk);
            }
            Ok(None) => return Ok(buf),
            Err(e) => return Err(ApiError::validation(format!("读取字段失败: {e}"))),
        }
    }
}
