//! 未安装门禁：除 /api/health、/api/install/status、POST /api/install 外，
//! 未安装状态下所有 /api/* 一律 503 not_installed。

use axum::extract::{Request, State};
use axum::middleware::Next;
use axum::response::Response;

use crate::error::ApiError;
use crate::state::AppState;

/// 免安装白名单（精确路径匹配；/api/install 仅在 POST 时放行）
fn is_exempt(path: &str, method: &axum::http::Method) -> bool {
    path == "/api/health"
        || path == "/api/install/status"
        || (path == "/api/install" && method == axum::http::Method::POST)
}

pub async fn not_installed_gate(
    State(state): State<AppState>,
    req: Request,
    next: Next,
) -> Response {
    // 路由经 nest("/api", ..) 挂载，req.uri() 是剥掉前缀的路径；
    // 用 OriginalUri 取完整的 /api/... 路径来做白名单判断
    let path = req
        .extensions()
        .get::<axum::extract::OriginalUri>()
        .map(|o| o.0.path().to_string())
        .unwrap_or_else(|| req.uri().path().to_string());
    let method = req.method().clone();

    if !is_exempt(&path, &method) && !state.is_installed().await {
        return axum::response::IntoResponse::into_response(ApiError::not_installed());
    }

    next.run(req).await
}
