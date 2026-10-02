//! 插件管理 API（全部需 Bearer，扩展契约「插件管理 API」）：
//! GET/POST(multipart zip) /api/admin/plugins、GET/DELETE /:slug、POST /:slug/enable|disable

use axum::extract::multipart::MultipartRejection;
use axum::extract::{Multipart, Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use serde_json::{json, Value};

use crate::error::{ApiError, ApiResult};
use crate::plugins::PluginInfo;
use crate::state::{require_pool, AppState};

use super::helpers::check_auth;

/// 提取 multipart 中 file 字段的 zip 字节；非 multipart / 缺字段 → 422 invalid_package
pub(crate) async fn read_zip_field(
    multipart: Result<Multipart, MultipartRejection>,
) -> ApiResult<Vec<u8>> {
    let mut mp = multipart.map_err(|_| {
        ApiError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "invalid_package",
            "需要 multipart/form-data，字段 file = zip 包",
        )
    })?;
    while let Some(field) = mp
        .next_field()
        .await
        .map_err(|e| ApiError::new(StatusCode::UNPROCESSABLE_ENTITY, "invalid_package", format!("multipart 解析失败: {e}")))?
    {
        if field.name() == Some("file") {
            let bytes = field.bytes().await.map_err(|e| {
                ApiError::new(
                    StatusCode::UNPROCESSABLE_ENTITY,
                    "invalid_package",
                    format!("读取上传内容失败: {e}"),
                )
            })?;
            return Ok(bytes.to_vec());
        }
    }
    Err(ApiError::new(
        StatusCode::UNPROCESSABLE_ENTITY,
        "invalid_package",
        "缺少 file 字段",
    ))
}

/// GET /api/admin/plugins → {"items":[PluginInfo], "total":int}
pub async fn admin_list_plugins(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> ApiResult<Json<Value>> {
    check_auth(&state, &headers).await?;
    let items: Vec<PluginInfo> = state.plugins().list().await;
    Ok(Json(json!({ "items": items, "total": items.len() as i64 })))
}

/// POST /api/admin/plugins（multipart file=zip）→ 201 PluginInfo（默认 enabled:false）
pub async fn admin_install_plugin(
    State(state): State<AppState>,
    headers: HeaderMap,
    multipart: Result<Multipart, MultipartRejection>,
) -> ApiResult<(StatusCode, Json<PluginInfo>)> {
    check_auth(&state, &headers).await?;
    let (pool, _db_type) = require_pool(&state).await?;
    let data = read_zip_field(multipart).await?;
    let info = state.plugins().install_from_zip(&pool, &data).await?;
    Ok((StatusCode::CREATED, Json(info)))
}

/// GET /api/admin/plugins/:slug → PluginInfo（含 last_error）
pub async fn admin_get_plugin(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(slug): Path<String>,
) -> ApiResult<Json<PluginInfo>> {
    check_auth(&state, &headers).await?;
    state
        .plugins()
        .get(&slug)
        .await
        .map(Json)
        .ok_or_else(ApiError::not_found)
}

/// POST /api/admin/plugins/:slug/enable → 200 PluginInfo（脚本语法错误 422 script_error）
pub async fn admin_enable_plugin(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(slug): Path<String>,
) -> ApiResult<Json<PluginInfo>> {
    check_auth(&state, &headers).await?;
    let (pool, _db_type) = require_pool(&state).await?;
    state.plugins().enable(&pool, &slug).await.map(Json)
}

/// POST /api/admin/plugins/:slug/disable → 200 PluginInfo
pub async fn admin_disable_plugin(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(slug): Path<String>,
) -> ApiResult<Json<PluginInfo>> {
    check_auth(&state, &headers).await?;
    let (pool, _db_type) = require_pool(&state).await?;
    state.plugins().disable(&pool, &slug).await.map(Json)
}

/// DELETE /api/admin/plugins/:slug → 204（启用中也可直接删）
pub async fn admin_delete_plugin(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(slug): Path<String>,
) -> ApiResult<StatusCode> {
    check_auth(&state, &headers).await?;
    let (pool, _db_type) = require_pool(&state).await?;
    state.plugins().delete(&pool, &slug).await?;
    Ok(StatusCode::NO_CONTENT)
}
