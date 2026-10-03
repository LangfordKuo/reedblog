//! reedblog 后端库入口：路由组装、启动状态恢复、服务运行。

pub mod auth;
pub mod config;
pub mod error;
pub mod handlers;
pub mod middleware;
pub mod models;
pub mod packages;
pub mod pages;
pub mod plugins;
pub mod seed;
pub mod settings;
pub mod state;
pub mod theme_settings;
pub mod themes;
pub mod views;
pub mod widgets;

use axum::extract::{DefaultBodyLimit, Request, State};
use axum::http::{header, HeaderValue, Method};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::Router;
use std::net::SocketAddr;
use std::path::Path;
use tower_http::cors::CorsLayer;

use config::Config;
use error::ApiError;
use handlers::{
    admin_comments, admin_pages, admin_plugins, admin_posts, admin_terms, admin_themes, feed,
    frontend, install, public, site_auth, site_settings, uploads,
};
// handlers::pages 与领域模块 crate::pages 同名，导入时加别名区分
use handlers::pages as public_pages;
use state::{connect_pool, AppState};

/// 插件/主题 zip 上传的请求体上限
const UPLOAD_LIMIT: usize = 32 * 1024 * 1024;

/// 图片上传请求体上限的 multipart 开销余量（配置的文件上限之上再加 1MB）
const UPLOADS_OVERHEAD_SLACK: usize = 1024 * 1024;

/// /api/* 未匹配路径的兜底：未安装 → 503 not_installed；已安装 → 404 JSON
async fn api_fallback(State(state): State<AppState>, _req: Request) -> Response {
    if state.is_installed().await {
        ApiError::not_found().into_response()
    } else {
        ApiError::not_installed().into_response()
    }
}

/// 组装完整路由（含 CORS 与未安装门禁中间件）
pub fn build_router(state: AppState, allowed_origins: Vec<String>) -> Router {
    let origins: Vec<HeaderValue> = allowed_origins
        .iter()
        .filter_map(|s| s.parse::<HeaderValue>().ok())
        .collect();
    let cors = CorsLayer::new()
        .allow_origin(origins)
        .allow_methods([
            Method::GET,
            Method::POST,
            Method::PUT,
            Method::PATCH,
            Method::DELETE,
            Method::OPTIONS,
        ])
        .allow_headers([header::AUTHORIZATION, header::CONTENT_TYPE]);

    // 图片上传的请求体上限（构建时读一次 [uploads] max_size_mb；运行期 handler 内还会按最新配置逐块校验）
    let uploads_body_limit = state
        .uploads_max_size_bytes()
        .saturating_add(UPLOADS_OVERHEAD_SLACK);

    let api = Router::new()
        // 安装向导（未安装时也可用）
        .route("/health", get(install::health))
        .route("/install/status", get(install::install_status))
        .route("/install", post(install::install))
        // 站点公开接口
        .route("/site", get(site_auth::site_info))
        // 站点设置（公开；不进未安装门禁白名单，未安装 503）
        .route("/site/settings", get(site_settings::public_site_settings))
        // 站点统计（公开；站点信息组件数据源，未安装 503）
        .route("/site/stats", get(site_auth::site_stats))
        .route("/posts", get(public::list_posts))
        .route("/posts/{slug}", get(public::get_post))
        .route(
            "/posts/{slug}/comments",
            get(public::list_comments).post(public::create_comment),
        )
        // 点赞（公开；不进未安装门禁白名单，未安装 503——契约「浏览量与点赞」条款）
        .route(
            "/posts/{slug}/like",
            get(public::get_like)
                .post(public::like_post)
                .delete(public::unlike_post),
        )
        // 相关文章推荐（公开；不进未安装门禁白名单，未安装 503——契约「相关文章推荐」条款）
        .route("/posts/{slug}/related", get(public::list_related_posts))
        .route("/tags", get(public::list_tags))
        .route("/categories", get(public::list_categories))
        .route("/archive", get(public::archive))
        // 页面（公开；不进未安装门禁白名单，未安装 503）
        .route("/pages", get(public_pages::list_pages))
        .route("/pages/{slug}", get(public_pages::get_page))
        .route(
            "/pages/{slug}/comments",
            get(public_pages::list_page_comments).post(public_pages::create_page_comment),
        )
        // 全文搜索（已安装后公开，无需鉴权）
        .route("/search", get(public::search_posts))
        // 鉴权
        .route("/auth/login", post(site_auth::login))
        .route("/auth/me", get(site_auth::me))
        // 管理：文章
        .route(
            "/admin/posts",
            get(admin_posts::admin_list_posts).post(admin_posts::admin_create_post),
        )
        .route(
            "/admin/posts/{id}",
            get(admin_posts::admin_get_post)
                .put(admin_posts::admin_update_post)
                .delete(admin_posts::admin_delete_post),
        )
        // 行内快捷置顶/取消置顶（契约「文章置顶与定时发布」条款）
        .route(
            "/admin/posts/{id}/sticky",
            axum::routing::patch(admin_posts::admin_set_sticky),
        )
        // 管理：分类/标签
        .route(
            "/admin/categories",
            get(admin_terms::admin_list_categories).post(admin_terms::admin_create_category),
        )
        .route(
            "/admin/categories/{id}",
            axum::routing::put(admin_terms::admin_update_category)
                .delete(admin_terms::admin_delete_category),
        )
        .route(
            "/admin/tags",
            get(admin_terms::admin_list_tags).post(admin_terms::admin_create_tag),
        )
        .route(
            "/admin/tags/{id}",
            axum::routing::put(admin_terms::admin_update_tag).delete(admin_terms::admin_delete_tag),
        )
        // 管理：页面
        .route(
            "/admin/pages",
            get(admin_pages::admin_list_pages).post(admin_pages::admin_create_page),
        )
        .route(
            "/admin/pages/{id}",
            get(admin_pages::admin_get_page)
                .put(admin_pages::admin_update_page)
                .delete(admin_pages::admin_delete_page),
        )
        .route(
            "/admin/pages/{id}/toggle",
            axum::routing::patch(admin_pages::admin_toggle_page),
        )
        // 管理：评论
        .route("/admin/comments", get(admin_comments::admin_list_comments))
        .route(
            "/admin/comments/{id}",
            axum::routing::put(admin_comments::admin_update_comment)
                .delete(admin_comments::admin_delete_comment),
        )
        // 管理：站点设置
        .route(
            "/admin/site/settings",
            get(site_settings::admin_get_site_settings)
                .put(site_settings::admin_update_site_settings),
        )
        // 管理：图片上传（请求体上限 = 配置文件大小上限 + multipart 开销余量；
        // 超限文件由 handler 逐块计数报 422 file_too_large）
        .route(
            "/admin/uploads",
            post(uploads::admin_upload_image).layer(DefaultBodyLimit::max(uploads_body_limit)),
        )
        // 上传文件公开读取（已安装后无需鉴权；不进未安装门禁白名单）
        .route("/uploads/{*path}", get(uploads::serve_upload))
        // RSS feed 与 sitemap（已安装后公开）
        .route("/feed.xml", get(feed::feed_xml))
        .route("/sitemap.xml", get(feed::sitemap_xml))
        // 前端注入与主题（公开，未安装门禁白名单）
        .route("/frontend/injections", get(frontend::frontend_injections))
        .route("/themes/active", get(frontend::themes_active))
        .route("/themes/{slug}/theme.css", get(frontend::theme_css))
        .route("/themes/{slug}/preview.png", get(frontend::theme_preview))
        .route("/themes/{slug}/assets/{*path}", get(frontend::theme_asset))
        // 主题设置生效值（公开，未安装门禁白名单；未安装时 values=声明默认值）
        .route("/themes/{slug}/settings", get(frontend::theme_settings))
        // 主题组件生效配置（公开，未安装门禁白名单；未安装时=内置默认启用集）
        .route("/themes/{slug}/widgets", get(frontend::theme_widgets))
        // 管理：插件
        .route(
            "/admin/plugins",
            get(admin_plugins::admin_list_plugins)
                .post(admin_plugins::admin_install_plugin)
                .layer(DefaultBodyLimit::max(UPLOAD_LIMIT)),
        )
        .route(
            "/admin/plugins/{slug}",
            get(admin_plugins::admin_get_plugin).delete(admin_plugins::admin_delete_plugin),
        )
        .route(
            "/admin/plugins/{slug}/enable",
            post(admin_plugins::admin_enable_plugin),
        )
        .route(
            "/admin/plugins/{slug}/disable",
            post(admin_plugins::admin_disable_plugin),
        )
        // 管理：主题
        .route(
            "/admin/themes",
            get(admin_themes::admin_list_themes)
                .post(admin_themes::admin_install_theme)
                .layer(DefaultBodyLimit::max(UPLOAD_LIMIT)),
        )
        .route(
            "/admin/themes/{slug}",
            axum::routing::delete(admin_themes::admin_delete_theme),
        )
        .route(
            "/admin/themes/{slug}/activate",
            post(admin_themes::admin_activate_theme),
        )
        // 管理：主题设置（panel 只服务激活主题；PUT 按 slug 保存）
        .route(
            "/admin/themes/active/settings-panel",
            get(admin_themes::admin_active_settings_panel),
        )
        .route(
            "/admin/themes/{slug}/settings",
            axum::routing::put(admin_themes::admin_update_theme_settings),
        )
        // 管理：主题组件（GET 全量合并配置；PUT 全量替换保存）
        .route(
            "/admin/themes/{slug}/widgets",
            get(admin_themes::admin_get_theme_widgets).put(admin_themes::admin_put_theme_widgets),
        )
        // 未匹配路径兜底（layer 不覆盖默认 fallback，需显式声明）
        .fallback(api_fallback)
        // 未安装门禁：除白名单外一律 503 not_installed
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            middleware::not_installed_gate,
        ))
        .with_state(state);

    Router::new().nest("/api", api).layer(cors)
}

/// 启动时恢复状态：config.toml 可完整加载（含非空 jwt_secret）且数据库可连 → 已安装。
/// 无论是否已安装都补建内置 default 主题（幂等）；已安装则按 DB 恢复插件 enabled 状态。
pub async fn startup_state(config_path: &str) -> AppState {
    let state = AppState::new(config_path);
    // 首次运行/升级启动：themes/default 不存在时自动生成内置主题
    themes::ensure_default_theme(state.themes_dir());
    if let Some(cfg) = Config::load(Path::new(config_path)) {
        if cfg.auth.jwt_secret.is_empty() {
            return state;
        }
        match cfg.db_url() {
            Some(url) => match connect_pool(&cfg.database.db_type, &url).await {
                Ok(pool) => {
                    // 插件启用状态恢复（DB 无记录的磁盘插件视为未启用）
                    state.plugins().restore_from_db(&pool).await;
                    state
                        .activate(
                            cfg.database.db_type.clone(),
                            pool,
                            cfg.auth.jwt_secret.clone(),
                            cfg.site.title.clone(),
                            cfg.site.subtitle.clone().unwrap_or_default(),
                        )
                        .await;
                }
                Err(e) => {
                    eprintln!(
                        "[reedblog] config.toml 标记已安装，但数据库连接/迁移失败: {e}；\
                         以未安装状态启动"
                    );
                }
            },
            None => eprintln!("[reedblog] config.toml 中 db_type 无效，以未安装状态启动"),
        }
    }
    state
}

/// 运行 HTTP 服务（默认 127.0.0.1:3000，config.toml [server] 可改）
pub async fn run(config_path: &str) -> Result<(), Box<dyn std::error::Error>> {
    let cfg = Config::load(Path::new(config_path));
    let (host, port, origins) = match &cfg {
        Some(c) => (
            c.server.host.clone(),
            c.server.port,
            c.cors.allowed_origins.clone(),
        ),
        None => ("127.0.0.1".to_string(), 3000, config::default_origins()),
    };

    let state = startup_state(config_path).await;
    let app = build_router(state, origins);

    let addr = format!("{host}:{port}");
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    println!("[reedblog] 监听 http://{addr}（配置文件: {config_path}）");
    // ConnectInfo：浏览量去重的直连 IP 兜底来源（无反代头时取 TCP 对端地址；
    // 测试路径的裸 axum::serve 不提供，handler 侧 Option 兼容）
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await?;
    Ok(())
}
