//! 站点设置接口（契约「站点设置」条款）：
//! - GET /api/site/settings        → SiteSettingsPublic（公开，不含 base_url；未安装门禁 503）
//! - GET /api/admin/site/settings  → SiteSettingsAdmin（Bearer，全部字段）
//! - PUT /api/admin/site/settings  → SiteSettingsAdmin（Bearer，全量更新，校验失败 422）
//!
//! 进程内实时生效：读写都直达 settings 表，无内存缓存，修改后无需重启。

use axum::extract::State;
use axum::http::HeaderMap;
use axum::Json;

use crate::error::{ApiError, ApiResult, ValidJson};
use crate::models::{SiteSettingsAdmin, SiteSettingsBody, SiteSettingsPublic};
use crate::settings::{self, SiteSettings};
use crate::state::{require_pool, AppState};

use super::helpers::check_auth;

/// GET /api/site/settings → SiteSettingsPublic（前台头部/页脚/分页默认值渲染用）
pub async fn public_site_settings(
    State(state): State<AppState>,
) -> ApiResult<Json<SiteSettingsPublic>> {
    let (pool, _db_type) = require_pool(&state).await?;
    let s = settings::load(&pool, &state).await?;
    Ok(Json(SiteSettingsPublic::from(&s)))
}

/// GET /api/admin/site/settings → SiteSettingsAdmin（含 base_url）
pub async fn admin_get_site_settings(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> ApiResult<Json<SiteSettingsAdmin>> {
    check_auth(&state, &headers).await?;
    let (pool, _db_type) = require_pool(&state).await?;
    let s = settings::load(&pool, &state).await?;
    Ok(Json(SiteSettingsAdmin::from(&s)))
}

/// PUT /api/admin/site/settings → 200 SiteSettingsAdmin（更新后的完整设置）
pub async fn admin_update_site_settings(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: ValidJson<SiteSettingsBody>,
) -> ApiResult<Json<SiteSettingsAdmin>> {
    check_auth(&state, &headers).await?;
    let (pool, _db_type) = require_pool(&state).await?;
    let Json(req) = body.map_err(ApiError::from)?;
    let next: SiteSettings = req.into_settings();
    settings::validate(&next)?;
    settings::save(&pool, &next).await?;
    Ok(Json(SiteSettingsAdmin::from(&next)))
}
