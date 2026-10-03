//! 主题管理 API（需 Bearer，扩展契约「主题管理 API」）：
//! GET/POST(multipart zip) /api/admin/themes、POST /:slug/activate、DELETE /:slug

use axum::extract::multipart::MultipartRejection;
use axum::extract::{Multipart, Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use serde_json::{json, Value};
use std::path::PathBuf;

use crate::config::Config;
use crate::error::{ApiError, ApiResult};
use crate::packages;
use crate::state::{require_pool, AppState};
use crate::themes::{self, ThemeInfo, ThemeManifest, BUILTIN_THEME_SLUG};

use super::admin_plugins::read_zip_field;
use super::helpers::check_auth;

fn invalid_manifest(msg: impl Into<String>) -> ApiError {
    ApiError::new(StatusCode::UNPROCESSABLE_ENTITY, "invalid_manifest", msg)
}

/// GET /api/admin/themes → {"items":[ThemeInfo], "total":int}
pub async fn admin_list_themes(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> ApiResult<Json<Value>> {
    check_auth(&state, &headers).await?;
    let active = state.active_theme_slug();
    let items = themes::list_themes(state.themes_dir(), &active);
    Ok(Json(json!({ "items": items, "total": items.len() as i64 })))
}

/// POST /api/admin/themes（multipart file=zip）→ 201 ThemeInfo
/// default 不可覆盖（409 builtin_protected）；slug 占用 409 theme_exists；
/// zip 非法 422 invalid_package；theme.toml 非法 422 invalid_manifest
pub async fn admin_install_theme(
    State(state): State<AppState>,
    headers: HeaderMap,
    multipart: Result<Multipart, MultipartRejection>,
) -> ApiResult<(StatusCode, Json<ThemeInfo>)> {
    check_auth(&state, &headers).await?;
    let data = read_zip_field(multipart).await?;
    let themes_dir: PathBuf = state.themes_dir().to_path_buf();

    let staging = packages::make_staging_dir(&themes_dir)?;
    let result = install_theme_inner(&themes_dir, &data, &staging);
    let _ = std::fs::remove_dir_all(&staging);
    let slug = result?;

    let active = state.active_theme_slug();
    let info = themes::theme_info(&themes_dir, &slug, &active)
        .ok_or_else(|| ApiError::internal("安装后读取主题失败"))?;
    Ok((StatusCode::CREATED, Json(info)))
}

fn install_theme_inner(
    themes_dir: &std::path::Path,
    data: &[u8],
    staging: &std::path::Path,
) -> ApiResult<String> {
    let root = packages::extract_single_root_zip(data, staging)?;
    let root_path = staging.join(&root);

    if !root_path.join("theme.toml").is_file() {
        return Err(ApiError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "invalid_package",
            "zip 根目录内缺少 theme.toml",
        ));
    }
    let text = std::fs::read_to_string(root_path.join("theme.toml"))
        .map_err(|e| invalid_manifest(format!("无法读取 theme.toml: {e}")))?;
    let manifest: ThemeManifest =
        toml::from_str(&text).map_err(|e| invalid_manifest(format!("theme.toml 解析失败: {e}")))?;
    manifest.validate(&root).map_err(invalid_manifest)?;

    let slug = manifest.slug.clone();
    // default 为内置主题，不可被上传覆盖
    if slug == BUILTIN_THEME_SLUG {
        return Err(ApiError::conflict(
            "builtin_protected",
            "内置 default 主题不可被上传覆盖",
        ));
    }
    if themes::theme_exists(themes_dir, &slug) {
        return Err(ApiError::conflict(
            "theme_exists",
            format!("主题 '{slug}' 已存在"),
        ));
    }

    std::fs::create_dir_all(themes_dir)?;
    std::fs::rename(&root_path, themes_dir.join(&slug))?;
    Ok(slug)
}

/// POST /api/admin/themes/:slug/activate → 200 ThemeInfo（写 config.toml [themes] active）
pub async fn admin_activate_theme(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(slug): Path<String>,
) -> ApiResult<Json<ThemeInfo>> {
    check_auth(&state, &headers).await?;
    if !packages::valid_slug(&slug) || !themes::theme_exists(state.themes_dir(), &slug) {
        return Err(ApiError::not_found());
    }
    // active 的权威来源是 config.toml：加载 → 改 active → 回写
    let config_path = std::path::Path::new(state.config_path());
    let mut cfg = Config::load(config_path)
        .ok_or_else(|| ApiError::internal("config.toml 不可读，无法保存激活主题"))?;
    cfg.themes.active = slug.clone();
    cfg.save(config_path)?;

    let info = themes::theme_info(state.themes_dir(), &slug, &slug)
        .ok_or_else(|| ApiError::internal("激活后读取主题失败"))?;
    Ok(Json(info))
}

/// DELETE /api/admin/themes/:slug → 204
/// builtin(default) 不可删 409 builtin_protected；当前激活主题不可删 409 theme_active
pub async fn admin_delete_theme(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(slug): Path<String>,
) -> ApiResult<StatusCode> {
    check_auth(&state, &headers).await?;
    // 仅依赖 require_pool 确认已安装（主题不入 DB）；池错误自然上抛
    let (_pool, _db_type) = require_pool(&state).await?;

    if slug == BUILTIN_THEME_SLUG {
        return Err(ApiError::conflict(
            "builtin_protected",
            "内置 default 主题不可删除",
        ));
    }
    if !packages::valid_slug(&slug) || !themes::theme_exists(state.themes_dir(), &slug) {
        return Err(ApiError::not_found());
    }
    if state.active_theme_slug() == slug {
        return Err(ApiError::conflict(
            "theme_active",
            "当前激活主题不可删除，请先切换到其他主题",
        ));
    }
    std::fs::remove_dir_all(themes::theme_dir(state.themes_dir(), &slug))?;
    Ok(StatusCode::NO_CONTENT)
}
