//! 邮件通知（契约「邮件通知（SMTP）」条款，2026-10-04 新增）：
//! 新评论/新回复创建成功后向管理员邮箱发通知邮件。
//!
//! 安全与投递语义（硬性约束）：
//! - 密码只从 config.toml `[smtp] password` 或环境变量 `REEDBLOG_SMTP_PASSWORD`
//!   读取（环境变量优先），绝不入库、绝不经 API 返回（接口只回 has_password）；
//! - 发送全部在 `tokio::spawn` 的后台任务里完成，评论请求只付出一次 clone+spawn 的成本，
//!   发送失败只记日志，绝不改变评论创建的响应；
//! - 连接/发送超时约 10 秒（transport timeout + 外层 tokio::time::timeout 双保险）；
//! - enabled=false 或配置不完整时静默跳过，不建立任何 SMTP 连接；
//! - 第一版 fire-and-forget，无持久化队列/重试队列（理由见契约）。
//!
//! 其余 SMTP 项（enabled/host/port/username/from_name/from_email/to_email/tls）
//! 存 settings 表，与站点设置共用同一套 upsert（crate::settings::set_value）。

use std::path::Path;
use std::time::Duration;

use axum::http::HeaderMap;
use lettre::message::{header::ContentType, Mailbox, Message};
use lettre::transport::smtp::authentication::Credentials;
use lettre::transport::smtp::AsyncSmtpTransport;
use lettre::{Address, AsyncTransport, Tokio1Executor};
use sqlx::AnyPool;

use crate::config::Config;
use crate::error::{ApiError, ApiResult};
use crate::handlers::feed::site_base_url;
use crate::state::{now_rfc3339, require_pool, AppState};

// settings 表键名（契约「邮件通知-配置来源」字段清单）
pub const KEY_ENABLED: &str = "smtp_enabled";
pub const KEY_HOST: &str = "smtp_host";
pub const KEY_PORT: &str = "smtp_port";
pub const KEY_USERNAME: &str = "smtp_username";
pub const KEY_FROM_NAME: &str = "smtp_from_name";
pub const KEY_FROM_EMAIL: &str = "smtp_from_email";
pub const KEY_TO_EMAIL: &str = "smtp_to_email";
pub const KEY_TLS: &str = "smtp_tls";

/// 密码环境变量（优先于 config.toml `[smtp] password`）
pub const PASSWORD_ENV: &str = "REEDBLOG_SMTP_PASSWORD";

pub const DEFAULT_PORT: i64 = 587;

pub const TLS_STARTTLS: &str = "starttls";
pub const TLS_IMPLICIT: &str = "implicit";
pub const TLS_NONE: &str = "none";

/// 连接与发送超时（契约：连接/发送各不超过约 10 秒）
pub const SMTP_TIMEOUT: Duration = Duration::from_secs(10);

/// SMTP 设置全集（密码不在其中——见模块头注释）
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SmtpSettings {
    pub enabled: bool,
    pub host: String,
    pub port: i64,
    pub username: String,
    pub from_name: String,
    pub from_email: String,
    pub to_email: String,
    pub tls: String,
}

impl Default for SmtpSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            host: String::new(),
            port: DEFAULT_PORT,
            username: String::new(),
            from_name: String::new(),
            from_email: String::new(),
            to_email: String::new(),
            tls: TLS_STARTTLS.to_string(),
        }
    }
}

/// 从 settings 表读取（旧库/未配置按键回退默认值，进程内实时生效）
pub async fn load(pool: &AnyPool) -> ApiResult<SmtpSettings> {
    let pairs = crate::settings::load_all(pool).await?;
    let get = |key: &str| -> Option<String> {
        pairs
            .iter()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value.clone())
    };
    let tls = get(KEY_TLS)
        .map(|v| v.trim().to_ascii_lowercase())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| TLS_STARTTLS.to_string());
    Ok(SmtpSettings {
        enabled: get(KEY_ENABLED).map(|v| v.trim() == "true").unwrap_or(false),
        host: get(KEY_HOST).unwrap_or_default(),
        // 存量非法值（理论上写不进来）回退默认端口，避免后续 as u16 出错
        port: get(KEY_PORT)
            .and_then(|v| v.trim().parse::<i64>().ok())
            .filter(|v| (1..=65535).contains(v))
            .unwrap_or(DEFAULT_PORT),
        username: get(KEY_USERNAME).unwrap_or_default(),
        from_name: get(KEY_FROM_NAME).unwrap_or_default(),
        from_email: get(KEY_FROM_EMAIL).unwrap_or_default(),
        to_email: get(KEY_TO_EMAIL).unwrap_or_default(),
        tls,
    })
}

/// 写入 settings 表（逐键 upsert，与站点设置同一套 SQL）
pub async fn save(pool: &AnyPool, s: &SmtpSettings) -> ApiResult<()> {
    let pairs: [(&str, String); 8] = [
        (KEY_ENABLED, s.enabled.to_string()),
        (KEY_HOST, s.host.clone()),
        (KEY_PORT, s.port.to_string()),
        (KEY_USERNAME, s.username.clone()),
        (KEY_FROM_NAME, s.from_name.clone()),
        (KEY_FROM_EMAIL, s.from_email.clone()),
        (KEY_TO_EMAIL, s.to_email.clone()),
        (KEY_TLS, s.tls.clone()),
    ];
    for (name, value) in pairs {
        crate::settings::set_value(pool, name, &value).await?;
    }
    Ok(())
}

/// 基本邮箱格式：`local@domain`，域名含 `.` 且无空白、无连续/首尾点。
/// 只做基本格式校验（契约明示「基本格式」，不做 RFC 5322 完整解析）
fn is_valid_email(v: &str) -> bool {
    if v.chars().any(|c| c.is_whitespace()) {
        return false;
    }
    let mut parts = v.splitn(2, '@');
    let (local, domain) = match (parts.next(), parts.next()) {
        (Some(l), Some(d)) => (l, d),
        _ => return false,
    };
    !local.is_empty()
        && !local.starts_with('.')
        && !local.ends_with('.')
        && !domain.is_empty()
        && !domain.contains('@')
        && domain.contains('.')
        && !domain.starts_with('.')
        && !domain.ends_with('.')
        && !domain.contains("..")
}

/// PUT 前校验（契约「邮件通知-管理接口」校验条款）；失败 → 422 validation_error
pub fn validate(s: &SmtpSettings) -> ApiResult<()> {
    if !matches!(s.tls.as_str(), TLS_STARTTLS | TLS_IMPLICIT | TLS_NONE) {
        return Err(ApiError::validation(format!(
            "tls 非法（须为 starttls、implicit 或 none）：{}",
            s.tls
        )));
    }
    if !(1..=65535).contains(&s.port) {
        return Err(ApiError::validation("端口 port 必须是 1~65535 的整数"));
    }
    fn len_ok(v: &str, max: usize, label: &str) -> ApiResult<()> {
        if v.chars().count() > max {
            return Err(ApiError::validation(format!(
                "{label} 过长（上限 {max} 字符）"
            )));
        }
        Ok(())
    }
    len_ok(&s.host, 255, "SMTP 主机")?;
    len_ok(&s.username, 255, "SMTP 用户名")?;
    len_ok(&s.from_name, 100, "发件人名")?;
    len_ok(&s.from_email, 255, "发件邮箱")?;
    len_ok(&s.to_email, 255, "收件邮箱")?;

    if !s.host.is_empty() && s.host.chars().any(|c| c.is_whitespace() || c == '/' || c == ':') {
        return Err(ApiError::validation(
            "SMTP 主机不能包含空白、'/' 或 ':'（只填主机名或 IP）",
        ));
    }
    if !s.from_email.is_empty() && !is_valid_email(&s.from_email) {
        return Err(ApiError::validation("发件邮箱不是合法的邮箱地址"));
    }
    if !s.to_email.is_empty() && !is_valid_email(&s.to_email) {
        return Err(ApiError::validation("收件邮箱不是合法的邮箱地址"));
    }
    if s.enabled {
        if s.host.is_empty() {
            return Err(ApiError::validation("启用邮件通知时 SMTP 主机不能为空"));
        }
        if s.from_email.is_empty() {
            return Err(ApiError::validation("启用邮件通知时发件邮箱不能为空"));
        }
        if s.to_email.is_empty() {
            return Err(ApiError::validation("启用邮件通知时收件邮箱不能为空"));
        }
    }
    Ok(())
}

/// 解析 SMTP 密码：环境变量 REEDBLOG_SMTP_PASSWORD 优先，其次 config.toml
/// `[smtp] password`；均未配置返回空串。**绝不写日志、绝不入库、绝不进响应**
pub fn resolve_password(state: &AppState) -> String {
    if let Ok(v) = std::env::var(PASSWORD_ENV) {
        let t = v.trim();
        if !t.is_empty() {
            return t.to_string();
        }
    }
    Config::load(Path::new(state.config_path()))
        .map(|c| c.smtp.password.trim().to_string())
        .unwrap_or_default()
}

/// 密码是否已配置（管理接口只暴露这个布尔）
pub fn has_password(state: &AppState) -> bool {
    !resolve_password(state).is_empty()
}

/// 发送前置条件（评论通知与测试接口共用）：
/// 配置不完整时返回明确原因（评论路径据此静默跳过，测试接口据此回 422）
pub fn readiness(s: &SmtpSettings, password: &str) -> Result<(), String> {
    if s.host.trim().is_empty() {
        return Err("SMTP 主机未配置".to_string());
    }
    if s.from_email.trim().is_empty() {
        return Err("发件邮箱未配置".to_string());
    }
    if s.to_email.trim().is_empty() {
        return Err("收件邮箱未配置".to_string());
    }
    if !s.username.trim().is_empty() && password.trim().is_empty() {
        return Err(format!(
            "SMTP 密码未配置（请在 config.toml 的 [smtp] password 或环境变量 {PASSWORD_ENV} 中配置）"
        ));
    }
    Ok(())
}

// ---------- 邮件构造与发送 ----------

fn build_message(smtp: &SmtpSettings, subject: &str, body: &str) -> Result<Message, String> {
    let from_addr: Address = smtp
        .from_email
        .parse()
        .map_err(|e| format!("发件邮箱非法: {e}"))?;
    let to_addr: Address = smtp
        .to_email
        .parse()
        .map_err(|e| format!("收件邮箱非法: {e}"))?;
    let from_name = smtp.from_name.trim();
    Message::builder()
        .from(Mailbox::new(
            if from_name.is_empty() {
                None
            } else {
                Some(from_name.to_string())
            },
            from_addr,
        ))
        .to(Mailbox::new(None, to_addr))
        .subject(subject)
        .header(ContentType::TEXT_PLAIN)
        .body(body.to_string())
        .map_err(|e| format!("邮件构造失败: {e}"))
}

fn build_transport(
    smtp: &SmtpSettings,
    password: &str,
) -> Result<AsyncSmtpTransport<Tokio1Executor>, String> {
    let host = smtp.host.trim();
    let builder = match smtp.tls.as_str() {
        TLS_IMPLICIT => AsyncSmtpTransport::<Tokio1Executor>::relay(host),
        TLS_NONE => Ok(AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(host)),
        // 其余（含存量异常值）按默认 starttls 处理；写库路径已被 validate 拦住
        _ => AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(host),
    }
    .map_err(|e| format!("SMTP 连接配置失败: {e}"))?;
    let mut builder = builder
        .port(smtp.port as u16)
        .timeout(Some(SMTP_TIMEOUT));
    if !smtp.username.trim().is_empty() {
        builder = builder.credentials(Credentials::new(
            smtp.username.trim().to_string(),
            password.to_string(),
        ));
    }
    Ok(builder.build())
}

/// 同步发送一封邮件（测试接口直接调用；评论通知在 spawn 的任务里调用）。
/// 成功/失败都会更新内存态 last_result；返回 Err 为失败原因摘要（**不含密码**）。
pub async fn send(
    state: &AppState,
    smtp: &SmtpSettings,
    password: &str,
    subject: &str,
    body: &str,
) -> Result<(), String> {
    let result = send_inner(smtp, password, subject, body).await;
    match &result {
        Ok(()) => state.set_mail_last_result(true, "发送成功"),
        Err(e) => state.set_mail_last_result(false, e),
    }
    result
}

async fn send_inner(
    smtp: &SmtpSettings,
    password: &str,
    subject: &str,
    body: &str,
) -> Result<(), String> {
    let message = build_message(smtp, subject, body)?;
    let transport = build_transport(smtp, password)?;
    // transport 自带 10 秒超时；外层再包一层兜底（DNS/建连等路径异常时也能收敛）
    match tokio::time::timeout(SMTP_TIMEOUT, transport.send(message)).await {
        Ok(Ok(_)) => Ok(()),
        Ok(Err(e)) => Err(format!("SMTP 发送失败: {e}")),
        Err(_) => Err(format!("SMTP 发送超时（{} 秒）", SMTP_TIMEOUT.as_secs())),
    }
}

/// 管理员测试邮件（同步等待，返回明确失败原因；写入 last_result）
pub async fn send_test(
    pool: &AnyPool,
    state: &AppState,
    smtp: &SmtpSettings,
    password: &str,
) -> Result<(), String> {
    let site_title = crate::settings::load(pool, state)
        .await
        .map(|s| s.title)
        .unwrap_or_else(|_| "reedblog".to_string());
    let subject = format!("[{site_title}] SMTP 测试邮件");
    let body = format!(
        "这是一封来自 reedblog 的 SMTP 测试邮件。\n\
         收到即表示邮件通知配置可用。\n\n\
         发送时间：{}\n\
         收件人：{}\n",
        now_rfc3339(),
        smtp.to_email
    );
    send(state, smtp, password, &subject, &body).await
}

// ---------- 评论通知 ----------

/// 新评论/新回复事件（写库成功后由 handler 组装）
#[derive(Debug, Clone)]
pub struct CommentNotice {
    /// "post" | "page"
    pub target_type: String,
    pub target_id: i64,
    pub slug: String,
    pub author_name: String,
    pub content: String,
    /// 楼中楼回复（parent_id 非空）
    pub is_reply: bool,
}

/// 评论创建成功后的通知入口：只做 clone + spawn，立即返回（绝不阻塞评论请求）
pub fn spawn_comment_notification(state: &AppState, headers: &HeaderMap, notice: CommentNotice) {
    let state = state.clone();
    let headers = headers.clone();
    tokio::spawn(async move {
        if let Err(reason) = notify_comment(&state, &headers, notice).await {
            eprintln!("[reedblog] 邮件通知发送失败: {reason}");
        }
    });
}

async fn notify_comment(
    state: &AppState,
    headers: &HeaderMap,
    notice: CommentNotice,
) -> Result<(), String> {
    let (pool, _db_type) = require_pool(state).await.map_err(|_| "站点未安装".to_string())?;
    let smtp = load(&pool).await.map_err(|e| e.message)?;
    if !smtp.enabled {
        // 契约：enabled=false 静默跳过，不建立任何 SMTP 连接
        return Ok(());
    }
    let password = resolve_password(state);
    if let Err(reason) = readiness(&smtp, &password) {
        // 契约：配置不完整静默跳过（只记一行日志便于排查）
        eprintln!("[reedblog] 邮件通知跳过（配置不完整）: {reason}");
        return Ok(());
    }

    let site = crate::settings::load(&pool, state).await.map_err(|e| e.message)?;
    // base_url 三级优先与 RSS/sitemap 相同（站点设置 → config.toml → 请求头）
    let base = site_base_url(&site.base_url, &state.configured_base_url(), headers);
    let title = target_title(&pool, &notice.target_type, notice.target_id)
        .await
        .unwrap_or_else(|| "（目标不存在或已删除）".to_string());
    let path = if notice.target_type == "page" {
        "pages"
    } else {
        "posts"
    };
    let url = format!("{base}/{path}/{}", urlencoding::encode(&notice.slug));
    let kind = if notice.is_reply { "新回复" } else { "新评论" };
    let target_label = if notice.target_type == "page" {
        "页面"
    } else {
        "文章"
    };
    let subject = format!("[{}] {kind}：{title}", site.title);
    let body = format!(
        "站点「{site_title}」收到{kind}。\n\n\
         {target_label}：{title}\n\
         链接：{url}\n\n\
         评论者：{author}\n\
         内容：\n{content}\n\n\
         ---\n\
         后台评论管理：{base}/admin/comments\n",
        site_title = site.title,
        author = notice.author_name,
        content = notice.content,
    );
    send(state, &smtp, &password, &subject, &body).await
}

/// 目标标题（文章/页面两表；查不到返回 None，不报错——通知尽力而为）
async fn target_title(pool: &AnyPool, target_type: &str, target_id: i64) -> Option<String> {
    let sql = if target_type == "page" {
        "SELECT title FROM pages WHERE id = ?"
    } else {
        "SELECT title FROM posts WHERE id = ?"
    };
    sqlx::query_scalar::<_, String>(sql)
        .bind(target_id)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> SmtpSettings {
        SmtpSettings {
            enabled: true,
            host: "smtp.example.com".to_string(),
            port: 587,
            username: "user@example.com".to_string(),
            from_name: "博客".to_string(),
            from_email: "blog@example.com".to_string(),
            to_email: "admin@example.com".to_string(),
            tls: TLS_STARTTLS.to_string(),
        }
    }

    #[test]
    fn accepts_valid_settings() {
        assert!(validate(&base()).is_ok());
        let mut s = base();
        s.enabled = false;
        s.host = String::new();
        s.from_email = String::new();
        s.to_email = String::new();
        assert!(validate(&s).is_ok(), "未启用时允许留空");
        s.tls = TLS_NONE.to_string();
        s.port = 1;
        assert!(validate(&s).is_ok());
        s.tls = TLS_IMPLICIT.to_string();
        s.port = 65535;
        assert!(validate(&s).is_ok());
    }

    #[test]
    fn rejects_invalid_settings() {
        let mut s = base();
        s.port = 0;
        assert!(validate(&s).is_err());
        s.port = 65536;
        assert!(validate(&s).is_err());

        let mut s = base();
        s.tls = "ssl".to_string();
        assert!(validate(&s).is_err());

        for bad in ["not-an-email", "a@b", "a b@c.com", "@example.com", "a@.com", "a@x..com"] {
            let mut s = base();
            s.to_email = bad.to_string();
            assert!(validate(&s).is_err(), "非法收件邮箱应被拒绝: {bad}");
        }

        let mut s = base();
        s.host = "http://smtp.example.com".to_string();
        assert!(validate(&s).is_err(), "主机带 scheme 应被拒绝");

        // enabled=true 时必填
        for mutate in [
            (|s: &mut SmtpSettings| s.host = String::new()) as fn(&mut SmtpSettings),
            |s: &mut SmtpSettings| s.from_email = String::new(),
            |s: &mut SmtpSettings| s.to_email = String::new(),
        ] {
            let mut s = base();
            mutate(&mut s);
            assert!(validate(&s).is_err(), "enabled=true 缺必填项应被拒绝");
        }
    }

    #[test]
    fn email_format_basics() {
        assert!(is_valid_email("a@b.co"));
        assert!(is_valid_email("first.last+tag@sub.example.com"));
        assert!(!is_valid_email("a@b"));
        assert!(!is_valid_email("a@@b.com"));
        assert!(!is_valid_email(""));
    }

    #[test]
    fn readiness_reports_missing_pieces() {
        let password = "secret";
        let mut s = base();
        assert!(readiness(&s, password).is_ok());

        s.host = String::new();
        assert!(readiness(&s, password).unwrap_err().contains("主机"));
        s = base();
        s.from_email = String::new();
        assert!(readiness(&s, password).unwrap_err().contains("发件邮箱"));
        s = base();
        s.to_email = String::new();
        assert!(readiness(&s, password).unwrap_err().contains("收件邮箱"));
        // username 非空但密码为空 → 提示密码未配置
        let s = base();
        let err = readiness(&s, "").unwrap_err();
        assert!(err.contains("密码未配置"), "{err}");
        // 无用户名（匿名/内网中继）不要求密码
        let mut s = base();
        s.username = String::new();
        assert!(readiness(&s, "").is_ok());
    }

    #[test]
    fn last_result_records_without_password() {
        let state = AppState::new("nonexistent-test-config.toml");
        assert!(state.mail_last_result().is_none(), "未发送过应为 None");
        state.set_mail_last_result(false, "SMTP 发送失败: connection refused");
        let r = state.mail_last_result().expect("应记录结果");
        assert!(!r.ok);
        assert!(r.message.contains("connection refused"));
        assert!(!r.message.contains("password"));
        assert!(!r.at.is_empty());
        state.set_mail_last_result(true, "发送成功");
        assert!(state.mail_last_result().unwrap().ok);
    }
}
