//! 评论审核方式（先发后审 / 先审后发）集成测试（api-contract.md「评论审核方式」条款，
//! 2026-10-04 新增）：
//! - `comment_moderation=pre` 时新评论（含留言板、回复）落库 status=pending：
//!   公开列表查不到、文章 comment_count 不变、SiteStats.comment_count 不变；
//!   管理接口 `?status=pending` 能查到
//! - `PUT {status:"approved"}`（后台「通过」）后公开可见且计数 +1；hidden 恢复逻辑不变
//! - 切回 `post` 后新评论立即 approved；**切换前后已存在的数据状态不变**
//! - PUT 非法枚举值 → 422 validation_error；旧库/无该键 → 默认 post（行为与现状一致）
//! - `pre` 模式下反滥用（蜜罐/黑名单/限流）行为不变（判定先于审核状态）
//! - 邮件文案差异由 mailer.rs 单测覆盖（`comment_notice_content_post_and_pre_modes`）；
//!   本文件聚焦可见性与计数口径

use reedblog_backend::antispam::AntiSpamConfig;
use reedblog_backend::state::AppState;
use serde_json::{json, Value};
use std::path::Path;
use std::time::Duration;

/// 每个请求分配唯一 XFF，避免命中「同 IP + 同目标 60 秒 1 条」限流（生产常量不变）
fn visitor_ip() -> String {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static N: AtomicUsize = AtomicUsize::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    format!("10.{}.{}.{}", (n / 62500) % 250, (n / 250) % 250, n % 250)
}

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

/// 宽松限流（内容规则/蜜罐/审核测试用；限流专项测试自行注入短窗口）
fn relaxed() -> AntiSpamConfig {
    AntiSpamConfig {
        comment_short_window: Duration::from_millis(50),
        comment_short_max: 50,
        comment_long_window: Duration::from_secs(10),
        comment_long_max: 100,
        ..AntiSpamConfig::default()
    }
}

async fn setup(dir: &Path, cfg: Option<AntiSpamConfig>) -> (reqwest::Client, String, String) {
    let base = spawn_server(dir.join("config.toml").to_str().unwrap(), cfg).await;
    let c = reqwest::Client::new();
    let r = c
        .post(format!("{base}/api/install"))
        .json(&json!({
            "db_type": "sqlite",
            "sqlite_path": dir.join("reedblog.db").to_str().unwrap(),
            "admin": {"username": "admin", "password": "secret123"},
            "site": {"title": "审核测试站"}
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

fn bearer(token: &str) -> String {
    format!("Bearer {token}")
}

async fn publish_post(c: &reqwest::Client, base: &str, token: &str, title: &str) -> String {
    let r = c
        .post(format!("{base}/api/admin/posts"))
        .header("Authorization", bearer(token))
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

/// PUT 站点设置（全量更新；缺省字段回退默认值，overrides 覆盖指定字段）
async fn put_settings(
    c: &reqwest::Client,
    base: &str,
    token: &str,
    overrides: Value,
) -> reqwest::Response {
    let mut body = json!({"title": "审核测试站", "per_page": 10});
    for (k, v) in overrides.as_object().unwrap() {
        body[k] = v.clone();
    }
    c.put(format!("{base}/api/admin/site/settings"))
        .header("Authorization", bearer(token))
        .json(&body)
        .send()
        .await
        .unwrap()
}

/// PUT 设置并要求 200，返回响应 JSON
async fn set_moderation(c: &reqwest::Client, base: &str, token: &str, value: &str) -> Value {
    let r = put_settings(c, base, token, json!({"comment_moderation": value})).await;
    assert_eq!(r.status(), 200, "切换审核方式应 200");
    r.json().await.unwrap()
}

/// 公开站点设置里的 comment_moderation
async fn public_moderation(c: &reqwest::Client, base: &str) -> String {
    let v: Value = c
        .get(format!("{base}/api/site/settings"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    v["comment_moderation"]
        .as_str()
        .unwrap_or_default()
        .to_string()
}

/// 发评论（唯一 XFF），返回 (status, body)
async fn post_comment(
    c: &reqwest::Client,
    base: &str,
    slug: &str,
    body: Value,
) -> (reqwest::StatusCode, Value) {
    let r = c
        .post(format!("{base}/api/posts/{slug}/comments"))
        .header("x-forwarded-for", visitor_ip())
        .json(&body)
        .send()
        .await
        .unwrap();
    let status = r.status();
    let v = r.json().await.unwrap();
    (status, v)
}

/// 发评论并要求 201，返回 CommentPub
async fn comment_ok(c: &reqwest::Client, base: &str, slug: &str, content: &str) -> Value {
    let (status, v) = post_comment(
        c,
        base,
        slug,
        json!({"author_name": "甲", "content": content}),
    )
    .await;
    assert_eq!(status, 201, "发评论应 201: {v}");
    v
}

/// 公开评论列表（内容数组）
async fn public_comments(c: &reqwest::Client, base: &str, slug: &str) -> Vec<Value> {
    let r = c
        .get(format!("{base}/api/posts/{slug}/comments"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    r.json().await.unwrap()
}

/// 文章详情里的 comment_count
async fn detail_comment_count(c: &reqwest::Client, base: &str, slug: &str) -> i64 {
    let v: Value = c
        .get(format!("{base}/api/posts/{slug}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    // PostDetail = PostPublic + 内容字段（serde flatten），comment_count 在顶层
    v["comment_count"]
        .as_i64()
        .unwrap_or_else(|| panic!("详情响应缺 comment_count: {v}"))
}

/// SiteStats.comment_count
async fn stats_comment_count(c: &reqwest::Client, base: &str) -> i64 {
    let v: Value = c
        .get(format!("{base}/api/site/stats"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    v["comment_count"].as_i64().unwrap()
}

/// 管理端评论列表（指定 status），返回 items 数组
async fn admin_comments(c: &reqwest::Client, base: &str, token: &str, status: &str) -> Vec<Value> {
    let r = c
        .get(format!(
            "{base}/api/admin/comments?status={status}&per_page=100"
        ))
        .header("Authorization", bearer(token))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200, "管理列表 status={status} 应 200");
    r.json::<Value>().await.unwrap()["items"]
        .as_array()
        .unwrap()
        .clone()
}

/// 管理端改状态（后台「通过」= approved）
async fn set_comment_status(
    c: &reqwest::Client,
    base: &str,
    token: &str,
    id: i64,
    status: &str,
) -> reqwest::Response {
    c.put(format!("{base}/api/admin/comments/{id}"))
        .header("Authorization", bearer(token))
        .json(&json!({"status": status}))
        .send()
        .await
        .unwrap()
}

/// 列表里按内容找一条评论
fn find_by_content<'a>(items: &'a [Value], content: &str) -> Option<&'a Value> {
    items.iter().find(|it| it["content"] == json!(content))
}

/// 直连 SQLite 执行一条写 SQL（模拟旧库 / 校验落库状态用）
async fn db_exec(url: &str, sql: &str) {
    sqlx::any::install_default_drivers();
    let pool = sqlx::AnyPool::connect(url).await.unwrap();
    sqlx::query(sql).execute(&pool).await.unwrap();
    pool.close().await;
}

fn sqlite_url_from_config(config_path: &str) -> String {
    reedblog_backend::config::Config::load(Path::new(config_path))
        .expect("config.toml 应已由安装流程写入")
        .db_url()
        .expect("db_type 应为 sqlite")
}

// ---------- 1. pre 模式：pending 不可见、不计数；通过后可见、计数 +1 ----------

#[tokio::test(flavor = "multi_thread")]
async fn pre_mode_pending_invisible_until_approved() {
    let tmp = tempfile::tempdir().unwrap();
    let (c, base, token) = setup(tmp.path(), Some(relaxed())).await;
    let slug = publish_post(&c, &base, &token, "pre-post").await;

    // 默认先发后审；切到 pre 后公开接口实时可见该值（前台表单据此提示）
    assert_eq!(public_moderation(&c, &base).await, "post");
    let v = set_moderation(&c, &base, &token, "pre").await;
    assert_eq!(v["comment_moderation"], "pre");
    assert_eq!(public_moderation(&c, &base).await, "pre");

    let stats_before = stats_comment_count(&c, &base).await;

    // 新评论：201 且响应形状不变（CommentPub，不含 status）
    let created = comment_ok(&c, &base, &slug, "待审核评论").await;
    let id = created["id"].as_i64().unwrap();
    assert!(id > 0);
    assert!(
        created.get("status").is_none(),
        "CommentPub 形状不得新增 status 字段: {created}"
    );

    // 公开可见性：列表查不到、文章 comment_count=0、SiteStats 不变
    assert!(public_comments(&c, &base, &slug).await.is_empty());
    assert_eq!(detail_comment_count(&c, &base, &slug).await, 0);
    assert_eq!(stats_comment_count(&c, &base).await, stats_before);

    // 后台 ?status=pending 能查到；?status=approved/?status=hidden 查不到
    let pending = admin_comments(&c, &base, &token, "pending").await;
    let row = find_by_content(&pending, "待审核评论").expect("待审列表应含该评论");
    assert_eq!(row["id"], json!(id));
    assert_eq!(row["status"], "pending");
    assert!(find_by_content(
        &admin_comments(&c, &base, &token, "approved").await,
        "待审核评论"
    )
    .is_none());
    assert!(find_by_content(
        &admin_comments(&c, &base, &token, "hidden").await,
        "待审核评论"
    )
    .is_none());

    // 后台「通过」= PUT status=approved → 公开可见、计数 +1
    let r = set_comment_status(&c, &base, &token, id, "approved").await;
    assert_eq!(r.status(), 200);
    assert_eq!(r.json::<Value>().await.unwrap()["status"], "approved");
    let list = public_comments(&c, &base, &slug).await;
    assert_eq!(list.len(), 1);
    assert_eq!(list[0]["content"], "待审核评论");
    assert_eq!(detail_comment_count(&c, &base, &slug).await, 1);
    assert_eq!(stats_comment_count(&c, &base).await, stats_before + 1);

    // hidden 恢复逻辑不变：隐藏 → 不可见/不计数；再通过 → 恢复
    assert_eq!(
        set_comment_status(&c, &base, &token, id, "hidden")
            .await
            .status(),
        200
    );
    assert!(public_comments(&c, &base, &slug).await.is_empty());
    assert_eq!(detail_comment_count(&c, &base, &slug).await, 0);
    assert_eq!(stats_comment_count(&c, &base).await, stats_before);
    assert_eq!(
        set_comment_status(&c, &base, &token, id, "approved")
            .await
            .status(),
        200
    );
    assert_eq!(public_comments(&c, &base, &slug).await.len(), 1);
    assert_eq!(stats_comment_count(&c, &base).await, stats_before + 1);
}

// ---------- 2. pre 模式：回复同样 pending；通过各自审核后随线程可见 ----------

#[tokio::test(flavor = "multi_thread")]
async fn pre_mode_replies_pending_and_thread_visibility() {
    let tmp = tempfile::tempdir().unwrap();
    let (c, base, token) = setup(tmp.path(), Some(relaxed())).await;
    let slug = publish_post(&c, &base, &token, "pre-reply").await;

    // post 模式先建顶级评论（approved，作为可回复的父楼）
    let top = comment_ok(&c, &base, &slug, "顶级评论").await;
    let top_id = top["id"].as_i64().unwrap();

    // 切到 pre 后回复也建为 pending
    set_moderation(&c, &base, &token, "pre").await;
    let (status, reply) = post_comment(
        &c,
        &base,
        &slug,
        json!({"author_name": "乙", "content": "回复内容", "parent_id": top_id}),
    )
    .await;
    assert_eq!(status, 201, "{reply}");
    let reply_id = reply["id"].as_i64().unwrap();
    assert_eq!(reply["parent_id"], json!(top_id));

    // 公开列表只看到顶级评论（pending 回复不出现，comment_count=1）
    let list = public_comments(&c, &base, &slug).await;
    assert_eq!(list.len(), 1);
    assert_eq!(list[0]["content"], "顶级评论");
    assert_eq!(detail_comment_count(&c, &base, &slug).await, 1);

    // 后台通过回复 → 线程内两条都可见，计数 +1
    assert_eq!(
        set_comment_status(&c, &base, &token, reply_id, "approved")
            .await
            .status(),
        200
    );
    let list = public_comments(&c, &base, &slug).await;
    assert_eq!(list.len(), 2);
    assert_eq!(detail_comment_count(&c, &base, &slug).await, 2);

    // pre 模式下不能回复 pending 父评论（父评论校验不受模式影响，仍 422）
    let pending_top = comment_ok(&c, &base, &slug, "另一条待审").await;
    let pid = pending_top["id"].as_i64().unwrap();
    let (status, err) = post_comment(
        &c,
        &base,
        &slug,
        json!({"author_name": "丙", "content": "回复待审父级", "parent_id": pid}),
    )
    .await;
    assert_eq!(status, 422, "{err}");
    assert_eq!(err["error"]["code"], "validation_error");
}

// ---------- 3. 切换模式不批量改动历史数据；切回 post 新评论立即 approved ----------

#[tokio::test(flavor = "multi_thread")]
async fn switching_mode_keeps_existing_rows_untouched() {
    let tmp = tempfile::tempdir().unwrap();
    let (c, base, token) = setup(tmp.path(), Some(relaxed())).await;
    let slug = publish_post(&c, &base, &token, "switch-post").await;

    // post 模式：A 创建即 approved
    let a = comment_ok(&c, &base, &slug, "评论A").await;
    let a_id = a["id"].as_i64().unwrap();

    // 切 pre：B 为 pending；A 状态不变（仍公开可见）
    set_moderation(&c, &base, &token, "pre").await;
    let b = comment_ok(&c, &base, &slug, "评论B").await;
    let b_id = b["id"].as_i64().unwrap();
    let approved = admin_comments(&c, &base, &token, "approved").await;
    let row_a = approved
        .iter()
        .find(|it| it["id"] == json!(a_id))
        .expect("A 应仍为 approved");
    assert_eq!(row_a["status"], "approved");
    assert_eq!(public_comments(&c, &base, &slug).await.len(), 1);

    // 切回 post：C 立即 approved；A/B 状态保持原状（B 仍 pending）
    set_moderation(&c, &base, &token, "post").await;
    let c1 = comment_ok(&c, &base, &slug, "评论C").await;
    let c_id = c1["id"].as_i64().unwrap();
    let pending = admin_comments(&c, &base, &token, "pending").await;
    assert_eq!(
        find_by_content(&pending, "评论B").unwrap()["id"],
        json!(b_id)
    );
    assert!(find_by_content(&pending, "评论A").is_none());
    assert!(find_by_content(&pending, "评论C").is_none());
    let approved = admin_comments(&c, &base, &token, "approved").await;
    assert_eq!(
        find_by_content(&approved, "评论C").unwrap()["id"],
        json!(c_id)
    );
    assert!(find_by_content(&approved, "评论A").is_some());

    // 公开列表：A、C 可见，B 不可见
    let list = public_comments(&c, &base, &slug).await;
    let contents: Vec<&str> = list
        .iter()
        .map(|v| v["content"].as_str().unwrap())
        .collect();
    assert!(contents.contains(&"评论A") && contents.contains(&"评论C"));
    assert!(!contents.contains(&"评论B"));
    assert_eq!(detail_comment_count(&c, &base, &slug).await, 2);
}

// ---------- 4. 非法枚举值 422；旧库无该键默认 post ----------

#[tokio::test(flavor = "multi_thread")]
async fn invalid_values_rejected_and_missing_key_defaults_post() {
    let tmp = tempfile::tempdir().unwrap();
    let (c, base, token) = setup(tmp.path(), Some(relaxed())).await;
    let slug = publish_post(&c, &base, &token, "fallback-post").await;

    // 非法值 → 422 validation_error，且已存设置不被改动
    for bad in [json!("before"), json!("POST"), json!("")] {
        let r = put_settings(&c, &base, &token, json!({"comment_moderation": bad})).await;
        assert_eq!(r.status(), 422, "非法值应 422: {bad}");
        let v: Value = r.json().await.unwrap();
        assert_eq!(v["error"]["code"], "validation_error");
    }
    assert_eq!(public_moderation(&c, &base).await, "post");

    // 请求体缺失该字段 → 全量更新回默认 post
    let r = put_settings(&c, &base, &token, json!({})).await;
    assert_eq!(r.status(), 200);
    assert_eq!(
        r.json::<Value>().await.unwrap()["comment_moderation"],
        "post"
    );

    // 旧库/无该键：直连删掉 settings 行 → load 按键回退 post，行为与现状一致
    let url = sqlite_url_from_config(tmp.path().join("config.toml").to_str().unwrap());
    db_exec(
        &url,
        "DELETE FROM settings WHERE name = 'comment_moderation'",
    )
    .await;
    assert_eq!(public_moderation(&c, &base).await, "post");
    let created = comment_ok(&c, &base, &slug, "旧库评论").await;
    assert_eq!(public_comments(&c, &base, &slug).await.len(), 1);
    assert!(created["id"].as_i64().unwrap() > 0);

    // 列表接口非法 status → 422（校验扩展不放松）
    let r = c
        .get(format!("{base}/api/admin/comments?status=bogus"))
        .header("Authorization", bearer(&token))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 422);
    // PUT 非法 status → 422
    let r = set_comment_status(&c, &base, &token, created["id"].as_i64().unwrap(), "gone").await;
    assert_eq!(r.status(), 422);
}

// ---------- 5. pre 模式下反滥用行为不变（蜜罐/黑名单/限流先于审核状态） ----------

#[tokio::test(flavor = "multi_thread")]
async fn pre_mode_antispam_behavior_unchanged() {
    let tmp = tempfile::tempdir().unwrap();
    // 短窗口 1 条快速触发限流（生产阈值常量不变，仅测试注入）
    let cfg = AntiSpamConfig {
        comment_short_window: Duration::from_millis(500),
        comment_short_max: 1,
        comment_long_window: Duration::from_secs(10),
        comment_long_max: 5,
        ..AntiSpamConfig::default()
    };
    let (c, base, token) = setup(tmp.path(), Some(cfg)).await;
    let slug = publish_post(&c, &base, &token, "pre-spam").await;

    // pre + 关键词黑名单（判定顺序：蜜罐 → 黑名单 → 限流 → 落库）
    let r = put_settings(
        &c,
        &base,
        &token,
        json!({"comment_moderation": "pre", "comment_blocked_keywords": "内涵词"}),
    )
    .await;
    assert_eq!(r.status(), 200);

    let pending_before = admin_comments(&c, &base, &token, "pending").await.len();

    // 1) 蜜罐：仍 201 假成功（id=0）、不落库（pending 列表不变）
    let (status, v) = post_comment(
        &c,
        &base,
        "pre-spam",
        json!({"author_name": "bot", "content": "正常内容", "website": "http://spam.example"}),
    )
    .await;
    assert_eq!(status, 201, "{v}");
    assert_eq!(v["id"], json!(0), "蜜罐应假成功 id=0");
    assert_eq!(
        admin_comments(&c, &base, &token, "pending").await.len(),
        pending_before,
        "蜜罐不得落库"
    );

    // 2) 黑名单：仍 403 comment_rejected（固定文案），不落库
    let (status, v) = post_comment(
        &c,
        &base,
        "pre-spam",
        json!({"author_name": "甲", "content": "这里有内涵词"}),
    )
    .await;
    assert_eq!(status, 403, "{v}");
    assert_eq!(v["error"]["code"], "comment_rejected");
    assert_eq!(
        admin_comments(&c, &base, &token, "pending").await.len(),
        pending_before,
        "403 不得落库"
    );

    // 3) 通过闸门的评论仍按 pre 落 pending；随后同 IP 第二条 → 429 + Retry-After
    let ip = visitor_ip();
    let post = |content: &str| {
        let c = c.clone();
        let base = base.clone();
        let slug = slug.clone();
        let ip = ip.clone();
        let content = content.to_string();
        async move {
            c.post(format!("{base}/api/posts/{slug}/comments"))
                .header("x-forwarded-for", ip)
                .json(&json!({"author_name": "甲", "content": content}))
                .send()
                .await
                .unwrap()
        }
    };
    let r = post("限流前第一条").await;
    assert_eq!(r.status(), 201);
    let created = r.json::<Value>().await.unwrap();
    let id = created["id"].as_i64().unwrap();
    let pending = admin_comments(&c, &base, &token, "pending").await;
    let row = pending
        .iter()
        .find(|it| it["id"] == json!(id))
        .expect("通过闸门的评论应按 pre 落为 pending");
    assert_eq!(row["status"], "pending");

    let r = post("限流第二条").await;
    assert_eq!(r.status(), 429);
    let has_retry_after = r.headers().get("retry-after").is_some();
    assert_eq!(
        r.json::<Value>().await.unwrap()["error"]["code"],
        "too_many_requests"
    );
    assert!(has_retry_after, "429 必须带 Retry-After 头");

    // 公开列表仍不含任何 pending（含第一条）
    assert!(public_comments(&c, &base, &slug).await.is_empty());
}
