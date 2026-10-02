//! reedblog 后端库入口：路由组装、启动状态恢复、服务运行。

pub mod auth;
pub mod config;
pub mod error;
pub mod handlers;
pub mod middleware;
pub mod models;
pub mod state;

use axum::extract::{Request, State};
use axum::http::{header, HeaderValue, Method};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::Router;
use std::path::Path;
use tower_http::cors::CorsLayer;

use config::Config;
use error::ApiError;
use handlers::{admin_comments, admin_posts, admin_terms, install, public, site_auth};
use state::{connect_pool, AppState};

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
            Method::DELETE,
            Method::OPTIONS,
        ])
        .allow_headers([header::AUTHORIZATION, header::CONTENT_TYPE]);

    let api = Router::new()
        // 安装向导（未安装时也可用）
        .route("/health", get(install::health))
        .route("/install/status", get(install::install_status))
        .route("/install", post(install::install))
        // 站点公开接口
        .route("/site", get(site_auth::site_info))
        .route("/posts", get(public::list_posts))
        .route("/posts/{slug}", get(public::get_post))
        .route(
            "/posts/{slug}/comments",
            get(public::list_comments).post(public::create_comment),
        )
        .route("/tags", get(public::list_tags))
        .route("/categories", get(public::list_categories))
        .route("/archive", get(public::archive))
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
            axum::routing::put(admin_terms::admin_update_tag)
                .delete(admin_terms::admin_delete_tag),
        )
        // 管理：评论
        .route("/admin/comments", get(admin_comments::admin_list_comments))
        .route(
            "/admin/comments/{id}",
            axum::routing::put(admin_comments::admin_update_comment)
                .delete(admin_comments::admin_delete_comment),
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

/// 启动时恢复状态：config.toml 可完整加载（含非空 jwt_secret）且数据库可连 → 已安装
pub async fn startup_state(config_path: &str) -> AppState {
    let state = AppState::new(config_path);
    if let Some(cfg) = Config::load(Path::new(config_path)) {
        if cfg.auth.jwt_secret.is_empty() {
            return state;
        }
        match cfg.db_url() {
            Some(url) => match connect_pool(&cfg.database.db_type, &url).await {
                Ok(pool) => {
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
        None => (
            "127.0.0.1".to_string(),
            3000,
            config::default_origins(),
        ),
    };

    let state = startup_state(config_path).await;
    let app = build_router(state, origins);

    let addr = format!("{host}:{port}");
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    println!("[reedblog] 监听 http://{addr}（配置文件: {config_path}）");
    axum::serve(listener, app).await?;
    Ok(())
}
