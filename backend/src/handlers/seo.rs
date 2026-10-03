//! SEO / 分享元信息 HTML 路由（契约「SEO / 分享元信息」条款，2026-10-04 新增）：
//! - `GET /posts/:slug`、`GET /pages/:slug`（**非 /api** 顶层路由）→
//!   爬虫/社媒预览 UA 得最小 OG HTML，普通 UA 302 到站点根；
//!   文章不可见（草稿/未到点 scheduled/回收站，复用 VISIBLE_POST_SQL）/ 页面停用 → 404
//! - `GET /robots.txt`（**非 /api** 顶层路由）→ text/plain
//!
//! 未安装时与其余公开接口同口径 → 503 not_installed（不进未安装门禁白名单）。
//! 生产由 Nginx 按 UA 分流（见 deploy/nginx.conf.example）；真人流量不经过这里。
//! 渲染与转义实现在 `crate::seo`（纯逻辑，可单测）。

use axum::extract::{Path, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use sqlx::Row;

use crate::error::{ApiError, ApiResult};
use crate::seo;
use crate::settings::{self, SiteSettings};
use crate::state::{now_rfc3339, require_pool, AppState};

use super::feed::site_base_url;
use super::helpers::{derive_excerpt, md_to_plain_text, VISIBLE_POST_SQL};

/// 爬虫分流上下文（base URL + 站点设置，一次请求共用）
struct SeoContext {
    base: String,
    settings: SiteSettings,
}

/// 前置判定结果
enum Gate {
    /// 命中爬虫白名单：继续查库渲染
    Crawler(SeoContext),
    /// 已直接生成的响应（未命中白名单的 302）
    Respond(Response),
}

/// 302 到站点根（未命中爬虫白名单的真人请求；避免把裸 HTML 给真人）
fn redirect_home(base: &str) -> Response {
    let mut resp = StatusCode::FOUND.into_response();
    let location = HeaderValue::from_str(&format!("{}/", base.trim_end_matches('/')))
        .unwrap_or_else(|_| HeaderValue::from_static("/"));
    resp.headers_mut().insert(header::LOCATION, location);
    resp
}

/// 前置：未安装 503 → 读设置与 base URL（三级优先）→ UA 不命中直接 302
async fn prepare(state: &AppState, headers: &HeaderMap) -> ApiResult<Gate> {
    if !state.is_installed().await {
        return Err(ApiError::not_installed());
    }
    let (pool, _db_type) = require_pool(state).await?;
    let settings = settings::load(&pool, state).await?;
    let base = site_base_url(&settings.base_url, &state.configured_base_url(), headers);
    let ua = headers
        .get(header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if !seo::is_crawler_ua(ua) {
        return Ok(Gate::Respond(redirect_home(&base)));
    }
    Ok(Gate::Crawler(SeoContext { base, settings }))
}

/// text/html 响应（OG HTML 统一 UTF-8）
fn html_response(html: String) -> Response {
    ([(header::CONTENT_TYPE, "text/html; charset=utf-8")], html).into_response()
}

/// og:image 选取链：正文第一张 Markdown 图片 → 站点设置 og_image → 省略（None）
fn pick_image(base: &str, content_md: &str, settings: &SiteSettings) -> Option<String> {
    seo::first_image_url(content_md)
        .and_then(|u| seo::absolutize(base, &u))
        .or_else(|| {
            let og = settings.og_image.trim();
            if og.is_empty() {
                None
            } else {
                seo::absolutize(base, og)
            }
        })
}

/// GET /posts/:slug → 最小 OG HTML（爬虫）/ 302 站点根（其余 UA）/ 404（不可见）
pub async fn post_html(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(slug): Path<String>,
) -> ApiResult<Response> {
    let ctx = match prepare(&state, &headers).await? {
        Gate::Crawler(ctx) => ctx,
        Gate::Respond(resp) => return Ok(resp),
    };
    let (pool, _db_type) = require_pool(&state).await?;
    // 可见性与公开详情同口径：published 或到点 scheduled，且不在回收站
    let sql = format!(
        "SELECT p.title, p.slug, p.excerpt, p.content_md, p.published_at, p.updated_at \
         FROM posts p WHERE p.slug = ? AND {VISIBLE_POST_SQL}"
    );
    let row = sqlx::query(&sql)
        .bind(&slug)
        .bind(now_rfc3339())
        .fetch_optional(&pool)
        .await?
        .ok_or_else(ApiError::not_found)?;

    let title = row.get::<String, _>("title");
    let excerpt = row.get::<String, _>("excerpt");
    let content_md = row.get::<String, _>("content_md");
    let published_at = row
        .try_get::<Option<String>, _>("published_at")
        .unwrap_or(None)
        .unwrap_or_default();
    let updated_at = row.get::<String, _>("updated_at");
    // description：excerpt 为空时按「excerpt 回退」同款规则从正文推导（纯文本）
    let description = if excerpt.trim().is_empty() {
        derive_excerpt(&content_md)
    } else {
        excerpt
    };

    let base = ctx.base.trim_end_matches('/');
    let canonical = format!("{}/posts/{}", base, urlencoding::encode(&slug));
    let image = pick_image(base, &content_md, &ctx.settings);
    let json_ld = seo::post_json_ld(&seo::PostJsonLd {
        title: &title,
        description: &description,
        canonical: &canonical,
        site_name: &ctx.settings.title,
        published_at: &published_at,
        updated_at: &updated_at,
        image: image.as_deref(),
        word_count: seo::word_count(&md_to_plain_text(&content_md)),
    });
    Ok(html_response(seo::render_og_html(&seo::OgPage {
        title: &title,
        description: &description,
        canonical: &canonical,
        base,
        site_name: &ctx.settings.title,
        og_type: "article",
        image: image.as_deref(),
        json_ld: &json_ld,
    })))
}

/// GET /pages/:slug → 最小 OG HTML（爬虫）/ 302 站点根（其余 UA）/ 404（停用/不存在）
pub async fn page_html(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(slug): Path<String>,
) -> ApiResult<Response> {
    let ctx = match prepare(&state, &headers).await? {
        Gate::Crawler(ctx) => ctx,
        Gate::Respond(resp) => return Ok(resp),
    };
    let (pool, _db_type) = require_pool(&state).await?;
    let row = sqlx::query(
        "SELECT title, slug, content_md, updated_at FROM pages WHERE slug = ? AND enabled = 1",
    )
    .bind(&slug)
    .fetch_optional(&pool)
    .await?
    .ok_or_else(ApiError::not_found)?;

    let title = row.get::<String, _>("title");
    let content_md = row.get::<String, _>("content_md");
    let updated_at = row.get::<String, _>("updated_at");
    // 页面无 excerpt 列：正文剥成纯文本后按同款规则截断
    let description = derive_excerpt(&content_md);

    let base = ctx.base.trim_end_matches('/');
    let canonical = format!("{}/pages/{}", base, urlencoding::encode(&slug));
    let image = pick_image(base, &content_md, &ctx.settings);
    let json_ld = seo::page_json_ld(&seo::PageJsonLd {
        title: &title,
        description: &description,
        canonical: &canonical,
        site_name: &ctx.settings.title,
        base,
        updated_at: &updated_at,
        image: image.as_deref(),
    });
    Ok(html_response(seo::render_og_html(&seo::OgPage {
        title: &title,
        description: &description,
        canonical: &canonical,
        base,
        site_name: &ctx.settings.title,
        og_type: "website",
        image: image.as_deref(),
        json_ld: &json_ld,
    })))
}

/// GET /robots.txt → text/plain（含后台 Disallow 与 sitemap 声明；base 同款三级优先）
pub async fn robots_txt(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> ApiResult<Response> {
    if !state.is_installed().await {
        return Err(ApiError::not_installed());
    }
    let (pool, _db_type) = require_pool(&state).await?;
    let settings = settings::load(&pool, &state).await?;
    let base = site_base_url(&settings.base_url, &state.configured_base_url(), &headers);
    Ok((
        [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
        seo::render_robots_txt(&base),
    )
        .into_response())
}
