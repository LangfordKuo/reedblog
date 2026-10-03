//! 邮件通知集成测试（契约「邮件通知（SMTP）」条款，2026-10-04 新增）：
//! - 管理接口鉴权：GET/PUT/POST test 未登录 → 401
//! - 配置读写：默认值、部分更新、GET 回读；**密码永不返回**、has_password 正确
//!   （config.toml [smtp] password 写入后即时为 true，响应体不含密码原文）
//! - 校验：端口范围 / tls 枚举 / 邮箱格式 / enabled 必填 → 422 validation_error
//! - 关键回归：SMTP 指向必然连不上的地址（127.0.0.1:1）时，
//!   POST /api/posts/:slug/comments 仍 201 且响应时间不受影响（带超时客户端断言）
//! - enabled=false 时完全不尝试发送（假 SMTP 服务器零连接）
//! - 假 SMTP 服务器下：测试邮件 202、文章评论与新回复各收到一封
//!   （收件人/主题/正文含链接与评论内容）；停掉假服务器后评论仍 201 且 last_result 记失败

use reedblog_backend::state::AppState;
use serde_json::{json, Value};
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};

// ---------- 测试脚手架（与 site_settings.rs 同款：随机端口真实服务 + tempdir 隔离） ----------

/// 反滥用限流（契约「反滥用」：同 IP + 同目标 60 秒 1 条）生效后，测试中连续发评论
/// 需模拟不同访客来源——每个请求分配唯一 XFF，避免命中限流返回 429。
fn visitor_ip() -> String {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static N: AtomicUsize = AtomicUsize::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    format!("10.{}.{}.{}", (n / 62500) % 250, (n / 250) % 250, n % 250)
}

async fn spawn_server(config_path: &str, base_url: Option<&str>) -> String {
    if !Path::new(config_path).exists() {
        let dir = Path::new(config_path).parent().unwrap();
        let toml_path = |p: std::path::PathBuf| p.to_str().unwrap().replace('\\', "/");
        let mut text = String::new();
        if let Some(b) = base_url {
            text.push_str(&format!("[server]\nbase_url = \"{b}\"\n\n"));
        }
        text.push_str(&format!(
            "[plugins]\ndir = \"{}\"\n\n[themes]\ndir = \"{}\"\n\n[uploads]\ndir = \"{}\"\n",
            toml_path(dir.join("plugins")),
            toml_path(dir.join("themes")),
            toml_path(dir.join("uploads")),
        ));
        std::fs::write(config_path, text).unwrap();
    }
    let state = AppState::new(config_path);
    let app = reedblog_backend::build_router(state, vec!["http://localhost:5173".to_string()]);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://127.0.0.1:{port}")
}

async fn err_code(r: reqwest::Response) -> String {
    let v: Value = r.json().await.unwrap();
    v["error"]["code"].as_str().unwrap_or_default().to_string()
}

async fn setup_installed(c: &reqwest::Client, base: &str, dir: &Path) -> String {
    let r = c
        .post(format!("{base}/api/install"))
        .json(&json!({
            "db_type": "sqlite",
            "sqlite_path": dir.join("reedblog.db").to_str().unwrap(),
            "admin": {"username": "admin", "password": "secret123"},
            "site": {"title": "SMTP Blog", "subtitle": ""}
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 201, "install 应成功");
    let r = c
        .post(format!("{base}/api/auth/login"))
        .json(&json!({"username": "admin", "password": "secret123"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    r.json::<Value>().await.unwrap()["token"]
        .as_str()
        .unwrap()
        .to_string()
}

fn bearer(token: &str) -> String {
    format!("Bearer {token}")
}

/// 管理员建一篇已发布文章（返回 slug）
async fn create_post(c: &reqwest::Client, base: &str, token: &str, slug: &str) -> String {
    let r = c
        .post(format!("{base}/api/admin/posts"))
        .header("Authorization", bearer(token))
        .json(&json!({
            "title": "SMTP Test Post",
            "slug": slug,
            "content_md": "body for smtp test",
            "status": "published"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 201, "建文章应成功");
    r.json::<Value>().await.unwrap()["slug"]
        .as_str()
        .unwrap()
        .to_string()
}

/// 更新 SMTP 设置（返回响应）
async fn put_smtp(
    c: &reqwest::Client,
    base: &str,
    token: &str,
    body: Value,
) -> reqwest::Response {
    c.put(format!("{base}/api/admin/smtp"))
        .header("Authorization", bearer(token))
        .json(&body)
        .send()
        .await
        .unwrap()
}

/// 一份可用的完整配置（指向给定端口的本地假服务器）
fn smtp_body(port: u16, enabled: bool) -> Value {
    json!({
        "enabled": enabled,
        "host": "127.0.0.1",
        "port": port,
        "username": "",
        "from_name": "SMTP Blog",
        "from_email": "blog@example.com",
        "to_email": "admin@example.com",
        "tls": "none"
    })
}

// ---------- 假 SMTP 服务器（EHLO/MAIL/RCPT/DATA/QUIT，无 AUTH） ----------

#[derive(Debug, Clone, Default)]
struct CapturedMail {
    from: String,
    recipients: Vec<String>,
    /// DATA 原文（含邮件头与正文）
    data: String,
}

struct FakeSmtp {
    port: u16,
    mails: Arc<Mutex<Vec<CapturedMail>>>,
    connections: Arc<AtomicUsize>,
    handle: tokio::task::JoinHandle<()>,
}

impl FakeSmtp {
    async fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let mails = Arc::new(Mutex::new(Vec::new()));
        let connections = Arc::new(AtomicUsize::new(0));
        let handle = tokio::spawn(accept_loop(
            listener,
            mails.clone(),
            connections.clone(),
        ));
        Self {
            port,
            mails,
            connections,
            handle,
        }
    }

    /// 轮询等待第 n 封邮件（最多 timeout）
    async fn wait_mails(&self, n: usize, timeout: Duration) -> Vec<CapturedMail> {
        let deadline = Instant::now() + timeout;
        loop {
            {
                let guard = self.mails.lock().unwrap();
                if guard.len() >= n {
                    return guard.clone();
                }
            }
            if Instant::now() > deadline {
                panic!(
                    "等待第 {n} 封邮件超时（已收到 {} 封）",
                    self.mails.lock().unwrap().len()
                );
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }
}

impl Drop for FakeSmtp {
    /// 停掉假服务器（abort 接收循环；已建立连接随任务结束关闭）
    fn drop(&mut self) {
        self.handle.abort();
    }
}

async fn accept_loop(
    listener: TcpListener,
    mails: Arc<Mutex<Vec<CapturedMail>>>,
    connections: Arc<AtomicUsize>,
) {
    while let Ok((stream, _)) = listener.accept().await {
        connections.fetch_add(1, Ordering::SeqCst);
        let mails = mails.clone();
        tokio::spawn(async move {
            let _ = handle_conn(stream, mails).await;
        });
    }
}

async fn handle_conn(stream: TcpStream, mails: Arc<Mutex<Vec<CapturedMail>>>) -> std::io::Result<()> {
    let (r, mut w) = stream.into_split();
    let mut reader = BufReader::new(r);
    w.write_all(b"220 reedblog-test ESMTP\r\n").await?;
    let mut mail = CapturedMail::default();
    let mut line = String::new();
    loop {
        line.clear();
        if reader.read_line(&mut line).await? == 0 {
            break;
        }
        let upper = line.trim_end().to_ascii_uppercase();
        if upper.starts_with("EHLO") || upper.starts_with("HELO") {
            w.write_all(b"250-reedblog-test\r\n250-8BITMIME\r\n250 OK\r\n")
                .await?;
        } else if upper.starts_with("MAIL FROM") {
            mail.from = line.trim_end().to_string();
            w.write_all(b"250 OK\r\n").await?;
        } else if upper.starts_with("RCPT TO") {
            mail.recipients.push(line.trim_end().to_string());
            w.write_all(b"250 OK\r\n").await?;
        } else if upper.starts_with("DATA") {
            w.write_all(b"354 End data with <CR><LF>.<CR><LF>\r\n").await?;
            let mut buf = Vec::new();
            loop {
                let mut b = [0u8; 1];
                if reader.read_exact(&mut b).await.is_err() {
                    break;
                }
                buf.push(b[0]);
                if buf.ends_with(b"\r\n.\r\n") {
                    break;
                }
            }
            mail.data = String::from_utf8_lossy(&buf).to_string();
            mails.lock().unwrap().push(std::mem::take(&mut mail));
            w.write_all(b"250 OK: queued\r\n").await?;
        } else if upper.starts_with("QUIT") {
            w.write_all(b"221 Bye\r\n").await?;
            break;
        } else {
            // RSET/NOOP/未知命令：宽容应答
            w.write_all(b"250 OK\r\n").await?;
        }
    }
    Ok(())
}

// ---------- 邮件解析小工具（测试专用：RFC2047 base64 主题 + Quoted-Printable 软换行） ----------

fn base64_decode(s: &str) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut acc: u32 = 0;
    let mut nbits = 0u32;
    let mut out = Vec::new();
    for b in s.bytes() {
        if b == b'=' {
            break;
        }
        let Some(i) = T.iter().position(|&c| c == b) else {
            continue;
        };
        acc = (acc << 6) | i as u32;
        nbits += 6;
        if nbits >= 8 {
            nbits -= 8;
            out.push((acc >> nbits) as u8);
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// 解码 RFC2047 主题（lettre 对非 ASCII 主题统一用 =?utf-8?b?...?=）
fn decode_rfc2047(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(start) = rest.find("=?utf-8?b?") {
        out.push_str(&rest[..start]);
        let after = &rest[start + "=?utf-8?b?".len()..];
        match after.find("?=") {
            Some(end) => {
                let b64: String = after[..end].chars().filter(|c| !c.is_whitespace()).collect();
                out.push_str(&base64_decode(&b64));
                rest = &after[end + 2..];
            }
            None => {
                out.push_str(&rest[start..]);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out
}

/// 取邮件头（支持折行）
fn header_value(raw: &str, name: &str) -> Option<String> {
    let prefix = format!("{name}:");
    let mut lines = raw.lines();
    while let Some(line) = lines.next() {
        if let Some(v) = line.strip_prefix(&prefix) {
            let mut value = v.trim().to_string();
            for l in lines.by_ref() {
                if l.starts_with(' ') || l.starts_with('\t') {
                    value.push_str(l.trim());
                } else {
                    break;
                }
            }
            return Some(value);
        }
    }
    None
}

/// 解码邮件正文：lettre 对含非 ASCII 的 text/plain 正文用 base64；
/// 纯 ASCII 时原样（或 Quoted-Printable 软换行）。两种都处理，便于断言中文内容。
fn decode_body(raw: &str) -> String {
    let body = match raw.split_once("\r\n\r\n") {
        Some((_, b)) => b,
        None => raw,
    };
    let cte = header_value(raw, "Content-Transfer-Encoding")
        .unwrap_or_default()
        .to_ascii_lowercase();
    if cte.contains("base64") {
        let compact: String = body.chars().filter(|c| !c.is_whitespace()).collect();
        base64_decode(&compact).trim_end_matches(['.', '\r', '\n']).to_string()
    } else {
        body.replace("=\r\n", "")
            .replace("=\n", "")
            .trim_end_matches(['.', '\r', '\n'])
            .to_string()
    }
}

// ---------- 1. 鉴权 ----------

#[tokio::test(flavor = "multi_thread")]
async fn smtp_admin_endpoints_require_auth() {
    let tmp = tempfile::tempdir().unwrap();
    let config = tmp.path().join("config.toml");
    let config = config.to_str().unwrap();
    let base = spawn_server(config, None).await;
    let c = reqwest::Client::new();
    // 先安装：未安装时未安装门禁对所有 /api/* 返回 503，会先于鉴权短路
    let _token = setup_installed(&c, &base, tmp.path()).await;

    let r = c.get(format!("{base}/api/admin/smtp")).send().await.unwrap();
    assert_eq!(r.status(), 401);
    assert_eq!(err_code(r).await, "unauthorized");

    let r = c
        .put(format!("{base}/api/admin/smtp"))
        .json(&json!({"enabled": false}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 401);

    let r = c
        .post(format!("{base}/api/admin/smtp/test"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 401);
}

// ---------- 2. 配置读写 / 密码处理 ----------

#[tokio::test(flavor = "multi_thread")]
async fn smtp_settings_roundtrip_and_password_never_returned() {
    let tmp = tempfile::tempdir().unwrap();
    let config = tmp.path().join("config.toml");
    let config = config.to_str().unwrap().to_string();
    let base = spawn_server(&config, None).await;
    let c = reqwest::Client::new();
    let token = setup_installed(&c, &base, tmp.path()).await;

    // 默认值（未配置时的回退）
    let v: Value = c
        .get(format!("{base}/api/admin/smtp"))
        .header("Authorization", bearer(&token))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(v["enabled"], json!(false));
    assert_eq!(v["port"], json!(587));
    assert_eq!(v["tls"], json!("starttls"));
    assert_eq!(v["has_password"], json!(false));
    assert_eq!(v["host"], json!(""));
    assert!(v["last_result"].is_null(), "从未发送过时 last_result 为 null");
    assert!(v.get("password").is_none(), "响应绝不能有 password 字段");

    // 部分更新：只给 8 个字段中的一部分也应成功（缺失保持原值）
    let r = put_smtp(&c, &base, &token, smtp_body(2525, true)).await;
    assert_eq!(r.status(), 200);
    let text = r.text().await.unwrap();
    assert!(!text.contains("\"password\""), "响应体不得含 password 字段: {text}");
    let v: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(v["enabled"], json!(true));
    assert_eq!(v["host"], json!("127.0.0.1"));
    assert_eq!(v["port"], json!(2525));
    assert_eq!(v["to_email"], json!("admin@example.com"));
    assert_eq!(v["has_password"], json!(false));

    // 部分更新：只改端口，其余字段保持
    let v: Value = put_smtp(&c, &base, &token, json!({"port": 2526}))
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(v["port"], json!(2526));
    assert_eq!(v["host"], json!("127.0.0.1"), "未提供的字段保持原值");
    assert_eq!(v["enabled"], json!(true));

    // GET 回读一致
    let v: Value = c
        .get(format!("{base}/api/admin/smtp"))
        .header("Authorization", bearer(&token))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(v["port"], json!(2526));
    assert_eq!(v["from_name"], json!("SMTP Blog"));

    // 密码只从 config.toml [smtp] password / 环境变量读取：写入后 has_password 翻转，
    // 且响应体绝不含密码原文。安装向导已写 [smtp] password = ""（字段序最后），
    // 这里截断到该段再重写（避免重复 table header）
    let text = std::fs::read_to_string(&config).unwrap();
    let idx = text.find("[smtp]").expect("安装后 config.toml 应含 [smtp] 段");
    std::fs::write(
        &config,
        format!("{}[smtp]\npassword = \"s3cret-pass\"\n", &text[..idx]),
    )
    .unwrap();
    let raw = c
        .get(format!("{base}/api/admin/smtp"))
        .header("Authorization", bearer(&token))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(!raw.contains("s3cret-pass"), "响应绝不能泄露密码: {raw}");
    let v: Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(v["has_password"], json!(true));
}

// ---------- 3. 校验 ----------

#[tokio::test(flavor = "multi_thread")]
async fn smtp_validation_returns_422() {
    let tmp = tempfile::tempdir().unwrap();
    let config = tmp.path().join("config.toml");
    let config = config.to_str().unwrap();
    let base = spawn_server(config, None).await;
    let c = reqwest::Client::new();
    let token = setup_installed(&c, &base, tmp.path()).await;

    let cases: Vec<(&str, Value)> = vec![
        ("端口为 0", json!({"port": 0})),
        ("端口越界", json!({"port": 65536})),
        ("tls 非枚举", json!({"tls": "ssl"})),
        ("收件邮箱非法", json!({"to_email": "not-an-email"})),
        ("发件邮箱非法", json!({"from_email": "a@b"})),
        ("主机含 scheme", json!({"host": "http://smtp.example.com"})),
        (
            "启用但主机为空",
            json!({"enabled": true, "host": "", "from_email": "a@b.com", "to_email": "c@d.com"}),
        ),
    ];
    for (label, body) in cases {
        let r = put_smtp(&c, &base, &token, body).await;
        assert_eq!(r.status(), 422, "{label} 应 422");
        assert_eq!(err_code(r).await, "validation_error", "{label}");
    }

    // 测试接口在配置不完整时也是 422（不是 502）
    let r = put_smtp(&c, &base, &token, json!({"host": "", "enabled": false})).await;
    assert_eq!(r.status(), 200);
    let r = c
        .post(format!("{base}/api/admin/smtp/test"))
        .header("Authorization", bearer(&token))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 422);
    assert_eq!(err_code(r).await, "validation_error");
}

// ---------- 4. 关键回归：连不上的 SMTP 不影响评论创建 ----------

#[tokio::test(flavor = "multi_thread")]
async fn unreachable_smtp_never_blocks_comment_creation() {
    let tmp = tempfile::tempdir().unwrap();
    let config = tmp.path().join("config.toml");
    let config = config.to_str().unwrap();
    let base = spawn_server(config, None).await;
    // 带超时的客户端：若评论请求被 SMTP 阻塞，这里会超时失败
    let c = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap();
    let token = setup_installed(&c, &base, tmp.path()).await;
    let slug = create_post(&c, &base, &token, "smtp-unreachable").await;

    // 必然连不上的地址（契约测试要求：127.0.0.1:1）
    let r = put_smtp(&c, &base, &token, smtp_body(1, true)).await;
    assert_eq!(r.status(), 200);

    let started = Instant::now();
    let r = c
        .post(format!("{base}/api/posts/{slug}/comments"))
        .header("x-forwarded-for", visitor_ip())
        .json(&json!({"author_name": "tester", "content": "unreachable smtp but comment ok"}))
        .send()
        .await
        .expect("评论请求不应被 SMTP 阻塞（客户端 5s 超时未触发）");
    let elapsed = started.elapsed();
    assert_eq!(r.status(), 201);
    let v: Value = r.json().await.unwrap();
    assert_eq!(v["content"], json!("unreachable smtp but comment ok"));
    assert!(
        elapsed < Duration::from_secs(1),
        "评论响应耗时应远小于 SMTP 超时（实测 {elapsed:?}）"
    );

    // 后台任务失败只记日志 → 内存态 last_result 记录失败原因
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let v: Value = c
            .get(format!("{base}/api/admin/smtp"))
            .header("Authorization", bearer(&token))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        if !v["last_result"].is_null() {
            assert_eq!(v["last_result"]["ok"], json!(false));
            let msg = v["last_result"]["message"].as_str().unwrap();
            assert!(msg.contains("SMTP"), "失败原因应可读: {msg}");
            break;
        }
        assert!(
            Instant::now() < deadline,
            "等待 last_result 记录发送失败超时"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

// ---------- 5. enabled=false 不尝试发送 ----------

#[tokio::test(flavor = "multi_thread")]
async fn disabled_smtp_never_attempts_send() {
    let tmp = tempfile::tempdir().unwrap();
    let config = tmp.path().join("config.toml");
    let config = config.to_str().unwrap();
    let base = spawn_server(config, None).await;
    let c = reqwest::Client::new();
    let token = setup_installed(&c, &base, tmp.path()).await;
    let fake = FakeSmtp::start().await;
    let slug = create_post(&c, &base, &token, "smtp-disabled").await;

    let r = put_smtp(&c, &base, &token, smtp_body(fake.port, false)).await;
    assert_eq!(r.status(), 200);

    let r = c
        .post(format!("{base}/api/posts/{slug}/comments"))
        .header("x-forwarded-for", visitor_ip())
        .json(&json!({"author_name": "tester", "content": "disabled smtp"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 201);

    tokio::time::sleep(Duration::from_millis(600)).await;
    assert_eq!(
        fake.connections.load(Ordering::SeqCst),
        0,
        "enabled=false 时不得建立任何 SMTP 连接"
    );
    assert!(fake.mails.lock().unwrap().is_empty());
}

// ---------- 6. 假 SMTP 服务器下的评论/回复通知 ----------

#[tokio::test(flavor = "multi_thread")]
async fn comment_and_reply_send_notifications() {
    let tmp = tempfile::tempdir().unwrap();
    let config = tmp.path().join("config.toml");
    let config = config.to_str().unwrap();
    // [server] base_url 固定，断言链接使用站点绝对 URL（base_url 三级优先的第一/二级）
    let base = spawn_server(config, Some("http://blog.example.test")).await;
    let c = reqwest::Client::new();
    let token = setup_installed(&c, &base, tmp.path()).await;
    let fake = FakeSmtp::start().await;
    let slug = create_post(&c, &base, &token, "smtp-notify").await;

    let r = put_smtp(&c, &base, &token, smtp_body(fake.port, true)).await;
    assert_eq!(r.status(), 200);

    // 顶级评论 → 一封「新评论」邮件
    let v: Value = c
        .post(format!("{base}/api/posts/{slug}/comments"))
        .header("x-forwarded-for", visitor_ip())
        .json(&json!({
            "author_name": "alice",
            "email": "alice@example.com",
            "content": "hello from smtp integration"
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let parent_id = v["id"].as_i64().unwrap();

    let mails = fake.wait_mails(1, Duration::from_secs(10)).await;
    let mail = &mails[0];
    assert!(
        mail.from.to_uppercase().contains("BLOG@EXAMPLE.COM"),
        "MAIL FROM 应含发件邮箱: {}",
        mail.from
    );
    assert!(
        mail.recipients
            .iter()
            .any(|r| r.to_uppercase().contains("ADMIN@EXAMPLE.COM")),
        "RCPT TO 应为 to_email: {:?}",
        mail.recipients
    );
    let subject = decode_rfc2047(&header_value(&mail.data, "Subject").expect("应有主题"));
    assert_eq!(subject, "[SMTP Blog] 新评论：SMTP Test Post");
    let body = decode_body(&mail.data);
    assert!(body.contains("SMTP Test Post"), "正文应含文章标题");
    assert!(
        body.contains(&format!("http://blog.example.test/posts/{slug}")),
        "正文应含站点绝对 URL 文章链接"
    );
    assert!(body.contains("hello from smtp integration"), "正文应含评论内容");
    assert!(body.contains("alice"), "正文应含评论者名");
    assert!(
        body.contains("http://blog.example.test/admin/comments"),
        "正文应含后台评论管理链接"
    );

    // 楼中楼回复 → 第二封「新回复」邮件
    let v: Value = c
        .post(format!("{base}/api/posts/{slug}/comments"))
        .header("x-forwarded-for", visitor_ip())
        .json(&json!({
            "author_name": "bob",
            "content": "reply from smtp integration",
            "parent_id": parent_id
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(v["parent_id"], json!(parent_id));

    let mails = fake.wait_mails(2, Duration::from_secs(10)).await;
    let mail = &mails[1];
    let subject = decode_rfc2047(&header_value(&mail.data, "Subject").expect("应有主题"));
    assert_eq!(subject, "[SMTP Blog] 新回复：SMTP Test Post");
    let body = decode_body(&mail.data);
    assert!(body.contains("reply from smtp integration"));
    assert!(body.contains("bob"));
}

// ---------- 6b. 页面留言（留言板）同样触发通知 ----------

#[tokio::test(flavor = "multi_thread")]
async fn page_message_sends_notification() {
    let tmp = tempfile::tempdir().unwrap();
    let config = tmp.path().join("config.toml");
    let config = config.to_str().unwrap();
    let base = spawn_server(config, Some("http://blog.example.test")).await;
    let c = reqwest::Client::new();
    let token = setup_installed(&c, &base, tmp.path()).await;
    let fake = FakeSmtp::start().await;

    let r = put_smtp(&c, &base, &token, smtp_body(fake.port, true)).await;
    assert_eq!(r.status(), 200);

    // 安装向导内置留言板（slug=guestbook，kind=message_board）
    let r = c
        .post(format!("{base}/api/pages/guestbook/comments"))
        .header("x-forwarded-for", visitor_ip())
        .json(&json!({"author_name": "carol", "content": "page message triggers smtp"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 201);

    let mails = fake.wait_mails(1, Duration::from_secs(10)).await;
    let subject = decode_rfc2047(&header_value(&mails[0].data, "Subject").expect("应有主题"));
    assert_eq!(subject, "[SMTP Blog] 新评论：留言板");
    let body = decode_body(&mails[0].data);
    assert!(
        body.contains("http://blog.example.test/pages/guestbook"),
        "正文应含页面绝对 URL：{body}"
    );
    assert!(body.contains("页面：留言板"));
    assert!(body.contains("page message triggers smtp"));
}

// ---------- 7. 测试邮件接口：假服务器下 202，停掉后 502 ----------

#[tokio::test(flavor = "multi_thread")]
async fn smtp_test_endpoint_success_and_failure() {
    let tmp = tempfile::tempdir().unwrap();
    let config = tmp.path().join("config.toml");
    let config = config.to_str().unwrap();
    let base = spawn_server(config, None).await;
    let c = reqwest::Client::new();
    let token = setup_installed(&c, &base, tmp.path()).await;
    let fake = FakeSmtp::start().await;

    let r = put_smtp(&c, &base, &token, smtp_body(fake.port, false)).await;
    assert_eq!(r.status(), 200);

    // 测试发送不要求 enabled=true
    let r = c
        .post(format!("{base}/api/admin/smtp/test"))
        .header("Authorization", bearer(&token))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 202);
    let v: Value = r.json().await.unwrap();
    assert_eq!(v["ok"], json!(true));

    let mails = fake.wait_mails(1, Duration::from_secs(10)).await;
    let subject = decode_rfc2047(&header_value(&mails[0].data, "Subject").expect("应有主题"));
    assert_eq!(subject, "[SMTP Blog] SMTP 测试邮件");

    // 成功也写入 last_result
    let v: Value = c
        .get(format!("{base}/api/admin/smtp"))
        .header("Authorization", bearer(&token))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(v["last_result"]["ok"], json!(true));

    // 停掉假服务器 → 502 smtp_send_failed，message 给明确原因
    drop(fake);
    let r = c
        .post(format!("{base}/api/admin/smtp/test"))
        .header("Authorization", bearer(&token))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 502);
    let v: Value = r.json().await.unwrap();
    assert_eq!(v["error"]["code"], json!("smtp_send_failed"));
    let msg = v["error"]["message"].as_str().unwrap();
    assert!(!msg.is_empty(), "失败必须给明确原因");
}
