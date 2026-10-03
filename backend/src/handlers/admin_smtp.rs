//! 邮件通知管理接口（契约「邮件通知（SMTP）」条款，2026-10-04 新增）：
//! - GET  /api/admin/smtp      → SmtpSettingsAdmin（**不含密码**，只有布尔 has_password）
//! - PUT  /api/admin/smtp      → 部分更新（缺失/null 字段保持原值），返回合并后的完整设置
//! - POST /api/admin/smtp/test → 202 {"ok": true}；同步等待发送结果，失败给明确原因
//!
//! 密码只从 config.toml `[smtp] password` 或环境变量 REEDBLOG_SMTP_PASSWORD 读取，
//! 绝不入库、绝不经本模块的任何响应返回。

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use serde_json::json;

use crate::error::{ApiError, ApiResult, ValidJson};
use crate::mailer;
use crate::models::{SmtpSettingsAdmin, SmtpSettingsBody};
use crate::state::{require_pool, AppState};

use super::helpers::check_auth;

/// 组装管理响应（密码→布尔 has_password；last_result 取内存态最近一次尝试）
fn admin_view(state: &AppState, s: &mailer::SmtpSettings) -> SmtpSettingsAdmin {
    SmtpSettingsAdmin {
        enabled: s.enabled,
        host: s.host.clone(),
        port: s.port,
        username: s.username.clone(),
        from_name: s.from_name.clone(),
        from_email: s.from_email.clone(),
        to_email: s.to_email.clone(),
        tls: s.tls.clone(),
        has_password: mailer::has_password(state),
        last_result: state.mail_last_result(),
    }
}

/// GET /api/admin/smtp → SmtpSettingsAdmin
pub async fn admin_get_smtp(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> ApiResult<Json<SmtpSettingsAdmin>> {
    check_auth(&state, &headers).await?;
    let (pool, _db_type) = require_pool(&state).await?;
    let smtp = mailer::load(&pool).await?;
    Ok(Json(admin_view(&state, &smtp)))
}

/// PUT /api/admin/smtp → 200 SmtpSettingsAdmin（部分更新后的完整设置）
pub async fn admin_update_smtp(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: ValidJson<SmtpSettingsBody>,
) -> ApiResult<Json<SmtpSettingsAdmin>> {
    check_auth(&state, &headers).await?;
    let (pool, _db_type) = require_pool(&state).await?;
    let Json(req) = body.map_err(ApiError::from)?;
    let current = mailer::load(&pool).await?;
    let next = req.apply_to(&current);
    mailer::validate(&next)?;
    mailer::save(&pool, &next).await?;
    Ok(Json(admin_view(&state, &next)))
}

/// POST /api/admin/smtp/test → 202 {"ok": true}
///
/// 同步等待发送结果（管理员主动点击，可接受等待；内部仍有约 10 秒超时兜底）：
/// - 配置不完整/非法 → 422 validation_error（带明确原因）
/// - SMTP 连接/发送失败 → 502 smtp_send_failed（message 为失败原因摘要，不含密码）
///
/// 不要求 enabled=true（便于先验证再启用），也不改变设置本身。
pub async fn admin_test_smtp(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> ApiResult<(StatusCode, Json<serde_json::Value>)> {
    check_auth(&state, &headers).await?;
    let (pool, _db_type) = require_pool(&state).await?;
    let smtp = mailer::load(&pool).await?;
    mailer::validate(&smtp)?;
    let password = mailer::resolve_password(&state);
    mailer::readiness(&smtp, &password).map_err(ApiError::validation)?;
    mailer::send_test(&pool, &state, &smtp, &password)
        .await
        .map_err(|e| ApiError::new(StatusCode::BAD_GATEWAY, "smtp_send_failed", e))?;
    Ok((StatusCode::ACCEPTED, Json(json!({ "ok": true }))))
}
