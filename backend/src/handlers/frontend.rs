//! 面向前端的公开端点（扩展契约：前端轻注入 + 主题应用机制）：
//! GET /api/frontend/injections、GET /api/themes/active、
//! GET /api/themes/:slug/theme.css、/:slug/preview.png、/:slug/assets/*path
//! 全部在未安装门禁白名单内（injections 未安装时返回空；active 兜底 default 令牌）。

use axum::extract::{Path, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::{json, Value};
use std::path::PathBuf;

use crate::packages;
use crate::themes;
use crate::state::AppState;

/// GET /api/frontend/injections → {"head":[{plugin,html}], "body_end":[...]}
/// 仅返回 enabled 且声明了对应 inject 的插件；未安装/无插件 → 空数组
pub async fn frontend_injections(State(state): State<AppState>) -> Json<Value> {
    let (head, body_end) = state.plugins().injections().await;
    Json(json!({
        "head": head.iter().map(|i| json!({"plugin": i.plugin, "html": i.html})).collect::<Vec<_>>(),
        "body_end": body_end.iter().map(|i| json!({"plugin": i.plugin, "html": i.html})).collect::<Vec<_>>(),
    }))
}

/// GET /api/themes/active → {slug, name, tokens, tokens_dark?, css_url|null, preview_url?}
/// 任何情况（含未安装）都能返回 default 主题令牌，保证前端永远有基础样式
pub async fn themes_active(State(state): State<AppState>) -> Json<Value> {
    let active = state.active_theme_slug();
    Json(themes::active_theme_response(state.themes_dir(), &active))
}

/// GET /api/themes/:slug/theme.css → text/css（不存在 404）
pub async fn theme_css(State(state): State<AppState>, Path(slug): Path<String>) -> Response {
    if !packages::valid_slug(&slug) {
        return StatusCode::NOT_FOUND.into_response();
    }
    let path = themes::theme_dir(state.themes_dir(), &slug).join("theme.css");
    match std::fs::read(&path) {
        Ok(body) => (
            [(header::CONTENT_TYPE, "text/css; charset=utf-8")],
            body,
        )
            .into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

/// GET /api/themes/:slug/preview.png → image/png（不存在 404）。
/// 契约中 ThemeInfo.preview_url 指向的托管端点（补充实现，契约未单列路由）
pub async fn theme_preview(State(state): State<AppState>, Path(slug): Path<String>) -> Response {
    if !packages::valid_slug(&slug) {
        return StatusCode::NOT_FOUND.into_response();
    }
    let path = themes::theme_dir(state.themes_dir(), &slug).join("preview.png");
    match std::fs::read(&path) {
        Ok(body) => ([(header::CONTENT_TYPE, "image/png")], body).into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

/// GET /api/themes/:slug/assets/*path → 静态文件（不存在 404；防目录穿越）
pub async fn theme_asset(
    State(state): State<AppState>,
    Path((slug, rel)): Path<(String, String)>,
) -> Response {
    if !packages::valid_slug(&slug) || rel.is_empty() {
        return StatusCode::NOT_FOUND.into_response();
    }
    // 词法清洗：拒绝绝对路径、`..`、反斜杠与盘符（%2e%2e 等解码后同样被拦截）
    let rel = rel.replace('\\', "/");
    let mut full = PathBuf::new();
    for seg in rel.split('/') {
        if seg.is_empty() || seg == "." {
            continue;
        }
        if seg == ".." || seg.contains(':') {
            return StatusCode::NOT_FOUND.into_response();
        }
        full.push(seg);
    }
    if full.as_os_str().is_empty() {
        return StatusCode::NOT_FOUND.into_response();
    }

    let assets = themes::theme_dir(state.themes_dir(), &slug).join("assets");
    let target = assets.join(&full);
    // 物理校验：canonicalize 后必须仍在 assets 目录内（防符号链接等残余手段）
    let (Ok(assets_canon), Ok(target_canon)) = (assets.canonicalize(), target.canonicalize())
    else {
        return StatusCode::NOT_FOUND.into_response();
    };
    if !target_canon.starts_with(assets_canon) || !target_canon.is_file() {
        return StatusCode::NOT_FOUND.into_response();
    }
    match std::fs::read(&target_canon) {
        Ok(body) => (
            [(header::CONTENT_TYPE, themes::mime_for(&rel))],
            body,
        )
            .into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}
