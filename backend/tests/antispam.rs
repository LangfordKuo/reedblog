//! 反滥用集成测试（api-contract.md「反滥用」条款，2026-10-04 新增）：
//! - 评论/留言限流：同 IP + 同目标超阈值 → 429 `too_many_requests` + `Retry-After`；
//!   短窗口（60s/1 条）与长窗口（10min/5 条）两个维度；不同 IP / 不同目标互不影响
//!   （测试通过 `AntiSpam::configure` 注入短窗口快速触发，不真等 60 秒）
//! - 蜜罐 `website` 非空 → 201 假成功（id=0）、不落库、不发通知、不占限流额度
//! - 关键词黑名单 / 链接数上限 → 403 `comment_rejected`（固定文案，不泄露规则）；
//!   两项设置仅后台可读写，公开接口不返回
//! - 留言板留言同样受保护（限流按目标分别计数）
//! - 后台登录：连续失败 5 次后锁定（即使密码正确也 429 `too_many_attempts` + `Retry-After`），
//!   成功登录清零计数；IP / 用户名维度隔离

use reedblog_backend::antispam::AntiSpamConfig;
use reedblog_backend::state::AppState;
use serde_json::{json, Value};
use std::path::Path;
use std::time::Duration;

/// 在 127.0.0.1 随机端口起真实服务；可注入反滥用短窗口（生产常量不变）
async fn spawn_server(config_path: &str, cfg: Option<AntiSpamConfig>) -> String {
    if !Path::new(config_path).exists() {
        let dir = Path::new(config_path).parent().unwrap();
        let toml_path = |p: std::path::PathBuf| p.to_str().unwrap().replace('\\', "/");
        std::fs::write(
            config_path,
            format!(
                "[plugins]\ndir = \"{}\"\n\n[themes]\ndir = \"{}\"\n\n[uploads]\ndir = \"{}\"\n",
                toml_path(dir.join("plugins")),
                toml_path(dir.join("themes")),
                toml_path(dir.join("uploads")),
            ),
        )
        .unwrap();
    }
    let state = AppState::new(config_path);
    if let Some(c) = cfg {
        state.antispam().configure(c);
    }
    let app = reedblog_backend::build_router(state, vec!["http://localhost:5173".to_string()]);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://127.0.0.1:{port}")
}

/// 短窗口快速轮转 + 长窗口宽松：用于内容规则/蜜罐测试（不触发限流）
fn relaxed() -> AntiSpamConfig {
    AntiSpamConfig {
        comment_short_window: Duration::from_millis(50),
        comment_short_max: 50,
        comment_long_window: Duration::from_secs(10),
        comment_long_max: 100,
        ..AntiSpamConfig::default()
    }
}

async fn err_code_msg(r: reqwest::Response) -> (String, String) {
    let v: Value = r.json().await.unwrap();
    (
        v["error"]["code"].as_str().unwrap_or_default().to_string(),
        v["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .to_string(),
    )
}

/// 安装 + 登录，返回 (client, base, token)
async fn setup(dir: &Path, cfg: Option<AntiSpamConfig>) -> (reqwest::Client, String, String) {
    let base = spawn_server(dir.join("config.toml").to_str().unwrap(), cfg).await;
    let c = reqwest::Client::new();
    let r = c
        .post(format!("{base}/api/install"))
        .json(&json!({
            "db_type": "sqlite",
            "sqlite_path": dir.join("reedblog.db").to_str().unwrap(),
            "admin": {"username": "admin", "password": "secret123"},
            "site": {"title": "反滥用测试站"}
        }))
        .send()
        .await
        .unwrap();
    let status = r.status();
    let text = r.text().await.unwrap();
    assert_eq!(status, 201, "install 失败: {text}");
    let r = c
        .post(format!("{base}/api/auth/login"))
        .json(&json!({"username": "admin", "password": "secret123"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    let token = r.json::<Value>().await.unwrap()["token"]
        .as_str()
        .unwrap()
        .to_string();
    (c, base, token)
}

/// 管理端发文（published），返回 slug
async fn publish_post(c: &reqwest::Client, base: &str, token: &str, title: &str) -> String {
    let r = c
        .post(format!("{base}/api/admin/posts"))
        .bearer_auth(token)
        .json(&json!({"title": title, "slug": title, "content_md": "正文", "status": "published"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 201);
    r.json::<Value>().await.unwrap()["slug"]
        .as_str()
        .unwrap()
        .to_string()
}

/// 发评论（可带 XFF 与 website 蜜罐），返回原始响应
async fn post_comment_raw(
    c: &reqwest::Client,
    base: &str,
    slug: &str,
    xff: &str,
    body: Value,
) -> reqwest::Response {
    c.post(format!("{base}/api/posts/{slug}/comments"))
        .header("x-forwarded-for", xff)
        .json(&body)
        .send()
        .await
        .unwrap()
}

async fn post_comment(
    c: &reqwest::Client,
    base: &str,
    slug: &str,
    xff: &str,
    body: Value,
) -> Value {
    let r = post_comment_raw(c, base, slug, xff, body).await;
    assert_eq!(r.status(), 201, "发评论应 201");
    r.json().await.unwrap()
}

fn retry_after(r: &reqwest::Response) -> u64 {
    r.headers()
        .get("retry-after")
        .expect("429 响应必须带 Retry-After 头")
        .to_str()
        .unwrap()
        .parse()
        .expect("Retry-After 应为整数秒")
}

// ---------- 1. 评论限流：短窗口 60s/1 条（注入短窗口快速触发） ----------

#[tokio::test(flavor = "multi_thread")]
async fn comment_rate_limit_short_window() {
    let tmp = tempfile::tempdir().unwrap();
    let cfg = AntiSpamConfig {
        comment_short_window: Duration::from_millis(400),
        comment_short_max: 1,
        comment_long_window: Duration::from_secs(10),
        comment_long_max: 5,
        ..AntiSpamConfig::default()
    };
    let (c, base, token) = setup(tmp.path(), Some(cfg)).await;
    publish_post(&c, &base, &token, "rate-post").await;
    publish_post(&c, &base, &token, "rate-post-2").await;

    // 第一条成功
    post_comment(
        &c,
        &base,
        "rate-post",
        "1.1.1.1",
        json!({"author_name": "甲", "content": "第一条"}),
    )
    .await;

    // 同 IP + 同目标立刻第二条 → 429 too_many_requests + Retry-After
    let r = post_comment_raw(
        &c,
        &base,
        "rate-post",
        "1.1.1.1",
        json!({"author_name": "甲", "content": "第二条"}),
    )
    .await;
    assert_eq!(r.status(), 429);
    let (code, msg) = err_code_msg(r).await;
    assert_eq!(code, "too_many_requests");
    assert!(
        !msg.contains("1 条") && !msg.contains("60") && !msg.contains("10 分钟"),
        "文案不得泄露具体规则: {msg}"
    );

    // 重新发一次取头部（上一步消费了响应体）
    let r = post_comment_raw(
        &c,
        &base,
        "rate-post",
        "1.1.1.1",
        json!({"author_name": "甲", "content": "第三条"}),
    )
    .await;
    assert_eq!(r.status(), 429);
    let ra = retry_after(&r);
    assert!(ra >= 1 && ra <= 60, "Retry-After 应为正秒数: {ra}");

    // 不同 IP → 不受影响
    post_comment(
        &c,
        &base,
        "rate-post",
        "2.2.2.2",
        json!({"author_name": "乙", "content": "换 IP"}),
    )
    .await;
    // 同 IP 不同目标（另一篇文章）→ 不受影响
    post_comment(
        &c,
        &base,
        "rate-post-2",
        "1.1.1.1",
        json!({"author_name": "甲", "content": "另一篇"}),
    )
    .await;

    // 短窗口（注入 400ms）过期后可再发
    tokio::time::sleep(Duration::from_millis(500)).await;
    post_comment(
        &c,
        &base,
        "rate-post",
        "1.1.1.1",
        json!({"author_name": "甲", "content": "窗口过期后"}),
    )
    .await;
}

// ---------- 2. 评论限流：长窗口 10min/5 条 ----------

#[tokio::test(flavor = "multi_thread")]
async fn comment_rate_limit_long_window() {
    let tmp = tempfile::tempdir().unwrap();
    let cfg = AntiSpamConfig {
        // 短窗口放到很短、额度很大 → 只由长窗口约束
        comment_short_window: Duration::from_millis(30),
        comment_short_max: 100,
        comment_long_window: Duration::from_secs(10),
        comment_long_max: 2,
        ..AntiSpamConfig::default()
    };
    let (c, base, token) = setup(tmp.path(), Some(cfg)).await;
    publish_post(&c, &base, &token, "long-post").await;

    for i in 0..2 {
        post_comment(
            &c,
            &base,
            "long-post",
            "9.9.9.9",
            json!({"author_name": "甲", "content": format!("第 {i} 条")}),
        )
        .await;
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    // 长窗口内第 3 条（短窗口早已轮转）→ 429，剩余时间接近长窗口
    let r = post_comment_raw(
        &c,
        &base,
        "long-post",
        "9.9.9.9",
        json!({"author_name": "甲", "content": "超长窗口"}),
    )
    .await;
    assert_eq!(r.status(), 429);
    let ra = retry_after(&r);
    assert!(ra >= 5, "剩余时间应由长窗口决定（≈10s）: {ra}");
    assert_eq!(err_code_msg(r).await.0, "too_many_requests");
}

// ---------- 3. 蜜罐：201 假成功、不落库、不发通知、不占额度 ----------

#[tokio::test(flavor = "multi_thread")]
async fn honeypot_fake_success_no_write_no_notification() {
    let tmp = tempfile::tempdir().unwrap();
    // 短窗口 5s/1 条：用于验证蜜罐不消耗限流额度（否则紧随其后的正常评论会被 429）
    let cfg = AntiSpamConfig {
        comment_short_window: Duration::from_secs(5),
        comment_short_max: 1,
        comment_long_window: Duration::from_secs(30),
        comment_long_max: 10,
        ..AntiSpamConfig::default()
    };
    let (c, base, token) = setup(tmp.path(), Some(cfg)).await;
    publish_post(&c, &base, &token, "honeypot-post").await;

    // 启用 SMTP 指向必然连不上的地址：若触发通知，后台任务会立刻把 last_result 记为失败
    let r = c
        .put(format!("{base}/api/admin/smtp"))
        .bearer_auth(&token)
        .json(&json!({
            "enabled": true, "host": "127.0.0.1", "port": 1,
            "from_email": "bot@example.com", "to_email": "admin@example.com",
            "username": "", "tls": "none"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);

    // 文章评论蜜罐：201 假成功，id=0，回显内容
    let fake = post_comment(
        &c,
        &base,
        "honeypot-post",
        "3.3.3.3",
        json!({"author_name": "机器人", "content": "买链接找我", "website": "http://spam.example"}),
    )
    .await;
    assert_eq!(fake["id"], 0, "蜜罐响应 id 为 0（假成功形状）");
    assert_eq!(fake["content"], "买链接找我");
    assert_eq!(fake["parent_id"], Value::Null);

    // 列表里查不到
    let list: Value = c
        .get(format!("{base}/api/posts/honeypot-post/comments"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(list.as_array().unwrap().is_empty(), "蜜罐评论不得落库");
    // 后台同样查不到（按目标文章 id 过滤，避开安装注入的示例评论）
    let post_id: i64 = c
        .get(format!("{base}/api/posts/honeypot-post"))
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap()["id"]
        .as_i64()
        .unwrap();
    let admin: Value = c
        .get(format!("{base}/api/admin/comments?post_id={post_id}"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(admin["total"], 0, "后台也查不到蜜罐评论");

    // 短暂等待：未触发任何通知（last_result 仍为 null）
    tokio::time::sleep(Duration::from_millis(400)).await;
    let v: Value = c
        .get(format!("{base}/api/admin/smtp"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(
        v["last_result"].is_null(),
        "蜜罐不得触发通知邮件: {}",
        v["last_result"]
    );

    // 蜜罐不占限流额度：紧接着的正常评论必须 201（若蜜罐计入则这里会 429）
    let real = post_comment(
        &c,
        &base,
        "honeypot-post",
        "3.3.3.3",
        json!({"author_name": "真人", "content": "正常评论"}),
    )
    .await;
    assert!(real["id"].as_i64().unwrap() > 0, "正常评论应有真实 id");

    // 正常评论会触发通知（证明上面的 null 不是因为 SMTP 路径整体失效）
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        let v: Value = c
            .get(format!("{base}/api/admin/smtp"))
            .bearer_auth(&token)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        if !v["last_result"].is_null() {
            assert_eq!(v["last_result"]["ok"], false);
            break;
        }
        assert!(std::time::Instant::now() < deadline, "等待通知尝试超时");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    // 留言板蜜罐同口径
    let fake = c
        .post(format!("{base}/api/pages/guestbook/comments"))
        .header("x-forwarded-for", "4.4.4.4")
        .json(&json!({"author_name": "机器人", "content": "spam", "website": "x"}))
        .send()
        .await
        .unwrap();
    assert_eq!(fake.status(), 201);
    assert_eq!(fake.json::<Value>().await.unwrap()["id"], 0);
    let v: Value = c
        .get(format!("{base}/api/pages/guestbook/comments"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(v.as_array().unwrap().is_empty(), "留言蜜罐不得落库");
}

// ---------- 4. 关键词黑名单 / 链接数上限：403 comment_rejected ----------

fn settings_body(keywords: &str, max_links: i64) -> Value {
    json!({
        "title": "反滥用测试站",
        "per_page": 10,
        "comment_blocked_keywords": keywords,
        "comment_max_links": max_links,
    })
}

#[tokio::test(flavor = "multi_thread")]
async fn keyword_and_link_blacklist() {
    let tmp = tempfile::tempdir().unwrap();
    let (c, base, token) = setup(tmp.path(), Some(relaxed())).await;
    publish_post(&c, &base, &token, "blacklist-post").await;

    // 写入设置（换行与逗号混用；大小写不敏感）
    let r = c
        .put(format!("{base}/api/admin/site/settings"))
        .bearer_auth(&token)
        .json(&settings_body("SpamWord\n广告词, 赌博", 3))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200, "设置应保存成功");
    let v: Value = r.json().await.unwrap();
    assert_eq!(v["comment_blocked_keywords"], "SpamWord\n广告词, 赌博");
    assert_eq!(v["comment_max_links"], 3);

    // 后台 GET 回读
    let v: Value = c
        .get(format!("{base}/api/admin/site/settings"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(v["comment_blocked_keywords"], "SpamWord\n广告词, 赌博");
    assert_eq!(v["comment_max_links"], 3);

    // 公开接口绝不返回这两项
    let v: Value = c
        .get(format!("{base}/api/site/settings"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(
        v.get("comment_blocked_keywords").is_none() && v.get("comment_max_links").is_none(),
        "公开设置不得泄露反滥用规则: {v}"
    );

    // 关键词命中（大小写不敏感）→ 403 comment_rejected，固定文案不泄露命中的词
    let r = post_comment_raw(
        &c,
        &base,
        "blacklist-post",
        "5.5.5.1",
        json!({"author_name": "甲", "content": "这里有 SPAMWORD 字样"}),
    )
    .await;
    assert_eq!(r.status(), 403);
    let (code, msg) = err_code_msg(r).await;
    assert_eq!(code, "comment_rejected");
    assert_eq!(msg, "内容未通过校验，请修改后重试");
    assert!(!msg.to_lowercase().contains("spamword") && !msg.contains("广告"));

    // 中文关键词命中
    let r = post_comment_raw(
        &c,
        &base,
        "blacklist-post",
        "5.5.5.2",
        json!({"author_name": "甲", "content": "广告词 出现在这里"}),
    )
    .await;
    assert_eq!(r.status(), 403);
    assert_eq!(err_code_msg(r).await.0, "comment_rejected");

    // 链接数 4 > 上限 3 → 403（文案同样不说是链接超限）
    let links4 = "看图 http://a.com http://b.com https://c.com https://d.com";
    let r = post_comment_raw(
        &c,
        &base,
        "blacklist-post",
        "5.5.5.3",
        json!({"author_name": "甲", "content": links4}),
    )
    .await;
    assert_eq!(r.status(), 403);
    let (code, msg) = err_code_msg(r).await;
    assert_eq!(code, "comment_rejected");
    assert!(!msg.contains("链接"), "文案不得暴露规则类型: {msg}");

    // 恰好 3 条链接 + 无关键词 → 201
    let links3 = "看图 http://a.com http://b.com https://c.com";
    post_comment(
        &c,
        &base,
        "blacklist-post",
        "5.5.5.4",
        json!({"author_name": "甲", "content": links3}),
    )
    .await;

    // 0 = 不限制：5 条链接放行
    let r = c
        .put(format!("{base}/api/admin/site/settings"))
        .bearer_auth(&token)
        .json(&settings_body("", 0))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    post_comment(
        &c,
        &base,
        "blacklist-post",
        "5.5.5.5",
        json!({"author_name": "甲", "content": "1 http://a.com 2 http://b.com 3 http://c.com 4 http://d.com 5 http://e.com"}),
    )
    .await;

    // 校验：max_links 越界 → 422
    for bad in [-1, 101] {
        let r = c
            .put(format!("{base}/api/admin/site/settings"))
            .bearer_auth(&token)
            .json(&settings_body("", bad))
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), 422, "comment_max_links={bad} 应 422");
        assert_eq!(err_code_msg(r).await.0, "validation_error");
    }
    // 全量更新语义：请求未带这两项 → 黑名单清空、链接数回默认 3
    let r = c
        .put(format!("{base}/api/admin/site/settings"))
        .bearer_auth(&token)
        .json(&json!({"title": "反滥用测试站", "per_page": 10}))
        .send()
        .await
        .unwrap();
    let v: Value = r.json().await.unwrap();
    assert_eq!(v["comment_blocked_keywords"], "");
    assert_eq!(v["comment_max_links"], 3, "缺失时回退默认值 3");
}

// ---------- 5. 页面留言同样受保护（限流按目标分别计数） ----------

#[tokio::test(flavor = "multi_thread")]
async fn page_comments_protected() {
    let tmp = tempfile::tempdir().unwrap();
    let cfg = AntiSpamConfig {
        comment_short_window: Duration::from_secs(5),
        comment_short_max: 1,
        comment_long_window: Duration::from_secs(30),
        comment_long_max: 5,
        ..AntiSpamConfig::default()
    };
    let (c, base, token) = setup(tmp.path(), Some(cfg)).await;
    publish_post(&c, &base, &token, "page-guard-post").await;

    // 黑名单对留言生效
    let r = c
        .put(format!("{base}/api/admin/site/settings"))
        .bearer_auth(&token)
        .json(&settings_body("留言黑词", 3))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);

    let post_page = |body: Value| {
        let c = c.clone();
        let base = base.clone();
        async move {
            c.post(format!("{base}/api/pages/guestbook/comments"))
                .header("x-forwarded-for", "7.7.7.7")
                .json(&body)
                .send()
                .await
                .unwrap()
        }
    };

    let r = post_page(json!({"author_name": "甲", "content": "留言黑词 出现"})).await;
    assert_eq!(r.status(), 403);
    assert_eq!(err_code_msg(r).await.0, "comment_rejected");

    // 第一条正常留言 → 201
    let r = post_page(json!({"author_name": "甲", "content": "正常留言"})).await;
    assert_eq!(r.status(), 201);

    // 同 IP 立刻第二条留言 → 429（留言目标限流）
    let r = post_page(json!({"author_name": "甲", "content": "第二条"})).await;
    assert_eq!(r.status(), 429);
    assert!(retry_after(&r) >= 1);

    // 同一 IP 发文章评论（不同目标）→ 不受留言限流影响
    let r = post_comment_raw(
        &c,
        &base,
        "page-guard-post",
        "7.7.7.7",
        json!({"author_name": "甲", "content": "文章评论"}),
    )
    .await;
    assert_eq!(r.status(), 201, "文章评论与留言板限流互不影响");

    // 留言蜜罐：201 假成功且不落库（换 IP 避开限流）
    let r = c
        .post(format!("{base}/api/pages/guestbook/comments"))
        .header("x-forwarded-for", "8.8.8.8")
        .json(&json!({"author_name": "机器人", "content": "spam", "website": "http://x.example"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 201);
    assert_eq!(r.json::<Value>().await.unwrap()["id"], 0);
    let v: Value = c
        .get(format!("{base}/api/pages/guestbook/comments"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(v.as_array().unwrap().len(), 1, "只有那条正常留言");
}

// ---------- 6. 后台登录失败退避：锁定与维度隔离 ----------

/// 登录请求：带 XFF 指定来源 IP
async fn login_req(base: &str, xff: &str, username: &str, password: &str) -> reqwest::Response {
    reqwest::Client::new()
        .post(format!("{base}/api/auth/login"))
        .header("x-forwarded-for", xff)
        .json(&json!({"username": username, "password": password}))
        .send()
        .await
        .unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn login_backoff_locks_after_five_failures() {
    let tmp = tempfile::tempdir().unwrap();
    let cfg = AntiSpamConfig {
        login_max_failures: 5,
        // 锁定期取得比「5 次 argon2 校验总耗时」更长，保证 5 次失败都落在同一窗口内；
        // 「锁定期满自动解锁」由 antispam 单元测试用毫秒级注入窗口覆盖
        login_lockout: Duration::from_secs(120),
        ..AntiSpamConfig::default()
    };
    let (_c, base, _token) = setup(tmp.path(), Some(cfg)).await;

    // 连续 5 次密码错误 → 401
    for i in 0..5 {
        let r = login_req(&base, "1.1.1.1", "admin", "wrong-password").await;
        assert_eq!(r.status(), 401, "第 {i} 次失败应 401");
        assert_eq!(err_code_msg(r).await.0, "invalid_credentials");
    }

    // 第 6 次即使密码正确 → 429 too_many_attempts + Retry-After
    let r = login_req(&base, "1.1.1.1", "admin", "secret123").await;
    assert_eq!(r.status(), 429, "锁定期间正确密码也应拒绝");
    assert_eq!(err_code_msg(r).await.0, "too_many_attempts");
    let r = login_req(&base, "1.1.1.1", "admin", "secret123").await;
    assert_eq!(r.status(), 429);
    let ra = retry_after(&r);
    assert!(ra >= 1 && ra <= 120, "Retry-After 应为剩余锁定时长: {ra}");

    // 维度隔离：换 IP / 换用户名均不受影响
    let r = login_req(&base, "2.2.2.2", "admin", "secret123").await;
    assert_eq!(r.status(), 200, "其他 IP 不受锁定影响");
    let r = login_req(&base, "1.1.1.1", "someone-else", "whatever").await;
    assert_eq!(r.status(), 401, "其他用户名不受锁定影响");
}

// ---------- 7. 后台登录失败退避：成功登录清零计数 ----------

#[tokio::test(flavor = "multi_thread")]
async fn login_success_clears_failures() {
    let tmp = tempfile::tempdir().unwrap();
    let cfg = AntiSpamConfig {
        login_max_failures: 5,
        // 锁定期远长于测试时长：排除「窗口过期」的干扰，单独验证清零语义
        login_lockout: Duration::from_secs(3600),
        ..AntiSpamConfig::default()
    };
    let (_c, base, _token) = setup(tmp.path(), Some(cfg)).await;

    // 失败 4 次（未到上限）→ 正确密码登录成功并清零
    for _ in 0..4 {
        let r = login_req(&base, "1.1.1.1", "admin", "wrong-password").await;
        assert_eq!(r.status(), 401);
    }
    let r = login_req(&base, "1.1.1.1", "admin", "secret123").await;
    assert_eq!(r.status(), 200, "未到上限时正确密码可登录");

    // 再失败 4 次 → 仍可登录（若未清零，此时已累计 8 次，第 6 次就该 429）
    for _ in 0..4 {
        let r = login_req(&base, "1.1.1.1", "admin", "wrong-password").await;
        assert_eq!(r.status(), 401);
    }
    let r = login_req(&base, "1.1.1.1", "admin", "secret123").await;
    assert_eq!(r.status(), 200, "成功登录应清零失败计数（第二次仍可登录）");

    // 清零后重新累计：连续 5 次失败 → 第 6 次正确密码也 429
    for _ in 0..5 {
        let r = login_req(&base, "1.1.1.1", "admin", "wrong-password").await;
        assert_eq!(r.status(), 401);
    }
    let r = login_req(&base, "1.1.1.1", "admin", "secret123").await;
    assert_eq!(r.status(), 429, "重新累计满 5 次后进入锁定");
    assert_eq!(err_code_msg(r).await.0, "too_many_attempts");
}
