//! SEO / 分享元信息集成测试（契约「SEO / 分享元信息」条款，2026-10-04 新增）：
//! - 爬虫/社媒预览 UA → 200 最小 OG HTML（og:title/og:description/canonical/JSON-LD/og:image）
//! - 普通浏览器 UA → 302 到站点根（后端不把裸 HTML 给真人）
//! - 草稿 / 未到点 scheduled / 回收站文章、停用页面 → 404
//! - og:image 选取链：正文第一张图 → 站点设置 og_image → 省略
//! - 标题/描述含 `<script>`/引号 → HTML 转义，JSON-LD 无原始 `<`
//! - /robots.txt：User-agent/Allow/Disallow/Sitemap
//! - og_image 设置写入/回读/非法值 422
//! - 未安装 → 503（与其余公开接口同口径）

use reedblog_backend::state::AppState;
use serde_json::{json, Value};
use std::path::Path;

const CRAWLER_UA: &str = "Twitterbot/1.0";
const BROWSER_UA: &str =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) \
     Chrome/126.0 Safari/537.36";
const BASE_URL: &str = "https://blog.example.com";

/// 在 127.0.0.1 随机端口起真实服务；预写 config.toml 把插件/主题/上传目录隔离到
/// tempdir，[server] base_url 固定为 BASE_URL（canonical 断言稳定）
async fn spawn_server(config_path: &str) -> String {
    if !Path::new(config_path).exists() {
        let dir = Path::new(config_path).parent().unwrap();
        let toml_path = |p: std::path::PathBuf| p.to_str().unwrap().replace('\\', "/");
        let text = format!(
            "[server]\nbase_url = \"{BASE_URL}\"\n\n[plugins]\ndir = \"{}\"\n\n\
             [themes]\ndir = \"{}\"\n\n[uploads]\ndir = \"{}\"\n",
            toml_path(dir.join("plugins")),
            toml_path(dir.join("themes")),
            toml_path(dir.join("uploads")),
        );
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

/// 不自动跟随重定向的客户端（302 断言用）
fn no_redirect_client() -> reqwest::Client {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap()
}

async fn setup_installed(c: &reqwest::Client, base: &str, dir: &Path) -> String {
    let r = c
        .post(format!("{base}/api/install"))
        .json(&json!({
            "db_type": "sqlite",
            "sqlite_path": dir.join("reedblog.db").to_str().unwrap(),
            "admin": {"username": "admin", "password": "secret123"},
            "site": {"title": "测试博客", "subtitle": "副标题"}
        }))
        .send()
        .await
        .unwrap();
    let status = r.status();
    if status != 201 {
        panic!("install 失败 {status}: {}", r.text().await.unwrap());
    }
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

async fn create_post(c: &reqwest::Client, base: &str, token: &str, body: Value) -> Value {
    let r = c
        .post(format!("{base}/api/admin/posts"))
        .bearer_auth(token)
        .json(&body)
        .send()
        .await
        .unwrap();
    let status = r.status();
    let v: Value = r.json().await.unwrap();
    assert_eq!(status, 201, "创建文章失败: {v}");
    v
}

async fn create_page(c: &reqwest::Client, base: &str, token: &str, body: Value) -> Value {
    let r = c
        .post(format!("{base}/api/admin/pages"))
        .bearer_auth(token)
        .json(&body)
        .send()
        .await
        .unwrap();
    let status = r.status();
    let v: Value = r.json().await.unwrap();
    assert_eq!(status, 201, "创建页面失败: {v}");
    v
}

/// 爬虫 UA 请求原始响应（状态码 + Content-Type + body；404 JSON 也能断言）
async fn crawler_get_raw(
    c: &reqwest::Client,
    url: String,
) -> (reqwest::StatusCode, String, String) {
    let r = c
        .get(url)
        .header("user-agent", CRAWLER_UA)
        .send()
        .await
        .unwrap();
    let status = r.status();
    let ct = r
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    (status, ct, r.text().await.unwrap())
}

/// 爬虫 UA 取 200 HTML 文本（断言状态码与 Content-Type）
async fn crawler_get(c: &reqwest::Client, url: String) -> (reqwest::StatusCode, String) {
    let (status, ct, body) = crawler_get_raw(c, url).await;
    assert_eq!(status, 200, "应 200: {body}");
    assert!(
        ct.starts_with("text/html"),
        "Content-Type 应为 text/html: {ct}"
    );
    assert!(ct.contains("charset=utf-8"), "应声明 UTF-8: {ct}");
    (status, body)
}

/// PUT 站点设置（全量；可选字段按需覆盖）
async fn put_settings(
    c: &reqwest::Client,
    base: &str,
    token: &str,
    overrides: Value,
) -> reqwest::Response {
    let mut body = json!({"title": "测试博客", "per_page": 10});
    for (k, v) in overrides.as_object().unwrap() {
        body[k] = v.clone();
    }
    c.put(format!("{base}/api/admin/site/settings"))
        .bearer_auth(token)
        .json(&body)
        .send()
        .await
        .unwrap()
}

// ---------- 1. 文章 OG HTML + 302 + 不可见 404 ----------

#[tokio::test(flavor = "multi_thread")]
async fn crawler_gets_og_html_and_browser_gets_redirect() {
    let dir = tempfile::tempdir().unwrap();
    let base = spawn_server(dir.path().join("config.toml").to_str().unwrap()).await;
    let c = reqwest::Client::new();
    let token = setup_installed(&c, &base, dir.path()).await;

    // 标题/描述含 <script> 与引号，正文第一张图在代码块之外
    let p = create_post(
        &c,
        &base,
        &token,
        json!({
            "title": "OG <script>alert(\"x\")</script> 测试",
            "slug": "og-post",
            "excerpt": "描述 & <b>加粗</b>",
            "content_md": "开头\n\n```\n![代码块里的图](in-code.png)\n```\n\n\
                           正文 ![首图](/api/uploads/ab/cd.png \"标题\")\n\n![次图](second.png)",
            "status": "published"
        }),
    )
    .await;
    let id = p["id"].as_i64().unwrap();

    // ---- 爬虫 UA：200 + OG HTML ----
    let (status, html) = crawler_get(&c, format!("{base}/posts/og-post")).await;
    assert_eq!(status, 200);
    // 标题/描述 HTML 转义（原始 <script> 不得出现）
    assert!(
        html.contains(
            "<title>OG &lt;script&gt;alert(&quot;x&quot;)&lt;/script&gt; 测试 - 测试博客</title>"
        ),
        "title 未按预期转义: {html}"
    );
    assert!(html.contains(r#"<meta property="og:title" content="OG &lt;script&gt;alert(&quot;x&quot;)&lt;/script&gt; 测试">"#));
    assert!(html.contains(
        r#"<meta property="og:description" content="描述 &amp; &lt;b&gt;加粗&lt;/b&gt;">"#
    ));
    assert!(!html.contains("<script>alert"), "原始 script 未转义");
    // canonical / og:url / og:type
    assert!(
        html.contains(r#"<link rel="canonical" href="https://blog.example.com/posts/og-post">"#)
    );
    // RSS 自动发现（非 JS 爬虫也能发现 feed）
    assert!(html.contains(
        r#"<link rel="alternate" type="application/rss+xml" href="https://blog.example.com/api/feed.xml">"#
    ));
    assert!(html
        .contains(r#"<meta property="og:url" content="https://blog.example.com/posts/og-post">"#));
    assert!(html.contains(r#"<meta property="og:type" content="article">"#));
    assert!(html.contains(r#"<meta property="og:site_name" content="测试博客">"#));
    // 正文第一张图（跳过代码块）成为 og:image，相对 URL 补绝对
    assert!(html.contains(
        r#"<meta property="og:image" content="https://blog.example.com/api/uploads/ab/cd.png">"#
    ));
    assert!(!html.contains("in-code.png"), "代码块内的图片不得被选中");
    assert!(!html.contains("second.png"), "只取第一张图");
    assert!(html.contains(r#"content="summary_large_image""#));
    // JSON-LD：BlogPosting，`<` 已转 unicode 转义
    assert!(html.contains(r#"<script type="application/ld+json">"#));
    assert!(html.contains(r#""@type":"BlogPosting""#));
    assert!(html.contains(r#""datePublished""#));
    assert!(html.contains(r#""wordCount""#));
    assert!(html.contains(r#""mainEntityOfPage""#));
    // JSON-LD 段落内不得出现原始 `<`（用户内容里的 <script> 已转 unicode 转义，防 </script> 截断）
    let ld = html
        .split(r#"<script type="application/ld+json">"#)
        .nth(1)
        .expect("应有 JSON-LD script");
    let ld = ld.split("</script>").next().unwrap();
    assert!(!ld.contains('<'), "JSON-LD 内出现原始 <: {ld}");
    assert!(!html.contains("</script>alert"));

    // ---- 普通浏览器 UA：302 到站点根 ----
    let c_nr = no_redirect_client();
    let r = c_nr
        .get(format!("{base}/posts/og-post"))
        .header("user-agent", BROWSER_UA)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 302, "普通 UA 应 302");
    assert_eq!(
        r.headers().get("location").unwrap().to_str().unwrap(),
        "https://blog.example.com/",
        "Location 应为站点根"
    );

    // ---- 草稿 / 未到点 scheduled / 回收站 → 404（爬虫 UA）----
    create_post(
        &c,
        &base,
        &token,
        json!({"title": "草稿", "slug": "og-draft", "content_md": "x", "status": "draft"}),
    )
    .await;
    let future = chrono::Utc::now() + chrono::Duration::days(3);
    create_post(
        &c,
        &base,
        &token,
        json!({
            "title": "未到点", "slug": "og-scheduled", "content_md": "x",
            "status": "scheduled", "published_at": future.to_rfc3339()
        }),
    )
    .await;
    let r = c
        .delete(format!("{base}/api/admin/posts/{id}"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 204, "软删（回收站）应成功");

    for slug in ["og-draft", "og-scheduled", "og-post"] {
        let (status, _, _) = crawler_get_raw(&c, format!("{base}/posts/{slug}")).await;
        assert_eq!(status, 404, "{slug} 对爬虫应 404（不可见）");
    }
    // 不存在的 slug 同样 404
    let (status, _, _) = crawler_get_raw(&c, format!("{base}/posts/no-such-post")).await;
    assert_eq!(status, 404);

    // 未命中 UA 的请求不做 404 判定：直接 302（避免泄露存在性）
    let r = c_nr
        .get(format!("{base}/posts/og-draft"))
        .header("user-agent", BROWSER_UA)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 302);
}

// ---------- 2. og:image 选取链 + og_image 设置读写/校验 ----------

#[tokio::test(flavor = "multi_thread")]
async fn og_image_fallback_chain_and_settings_validation() {
    let dir = tempfile::tempdir().unwrap();
    let base = spawn_server(dir.path().join("config.toml").to_str().unwrap()).await;
    let c = reqwest::Client::new();
    let token = setup_installed(&c, &base, dir.path()).await;

    create_post(
        &c,
        &base,
        &token,
        json!({"title": "无图文章", "slug": "no-image", "excerpt": "摘要", "content_md": "纯文字", "status": "published"}),
    )
    .await;

    // 站点 og_image 未设置 → 不含 og:image 且 twitter:card=summary
    let (status, html) = crawler_get(&c, format!("{base}/posts/no-image")).await;
    assert_eq!(status, 200);
    assert!(!html.contains("og:image"), "无任何图源时不应输出 og:image");
    assert!(!html.contains("twitter:image"));
    assert!(html.contains(r#"content="summary""#));

    // 非法 og_image → 422 validation_error
    for bad in ["javascript:alert(1)", "ftp://x.dev/a.png", "/uploads/a.png"] {
        let r = put_settings(&c, &base, &token, json!({"og_image": bad})).await;
        assert_eq!(r.status(), 422, "非法 og_image 应 422: {bad}");
        let v: Value = r.json().await.unwrap();
        assert_eq!(v["error"]["code"], "validation_error");
    }

    // 合法写入站内路径 → 回读，且回落到 og_image
    let r = put_settings(
        &c,
        &base,
        &token,
        json!({"og_image": "/api/uploads/fallback.png"}),
    )
    .await;
    assert_eq!(r.status(), 200);
    let v: Value = r.json().await.unwrap();
    assert_eq!(v["og_image"], "/api/uploads/fallback.png");
    // 公开读也带 og_image（无需鉴权）
    let v: Value = c
        .get(format!("{base}/api/site/settings"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(v["og_image"], "/api/uploads/fallback.png");

    let (status, html) = crawler_get(&c, format!("{base}/posts/no-image")).await;
    assert_eq!(status, 200);
    assert!(
        html.contains(r#"<meta property="og:image" content="https://blog.example.com/api/uploads/fallback.png">"#),
        "无正文图时应回落到站点 og_image: {html}"
    );
    assert!(html.contains(r#"content="summary_large_image""#));

    // 正文图优先于站点 og_image
    create_post(
        &c,
        &base,
        &token,
        json!({"title": "有图", "slug": "has-image", "content_md": "![图](https://cdn.example.com/a.jpg)", "status": "published"}),
    )
    .await;
    let (_, html) = crawler_get(&c, format!("{base}/posts/has-image")).await;
    assert!(html.contains(r#"content="https://cdn.example.com/a.jpg""#));
    assert!(
        !html.contains("fallback.png"),
        "正文图应优先于站点 og_image"
    );

    // 清空 og_image（空串允许）→ 无图文章不再含 og:image
    let r = put_settings(&c, &base, &token, json!({"og_image": ""})).await;
    assert_eq!(r.status(), 200);
    let (_, html) = crawler_get(&c, format!("{base}/posts/no-image")).await;
    assert!(!html.contains("og:image"));
}

// ---------- 3. robots.txt ----------

#[tokio::test(flavor = "multi_thread")]
async fn robots_txt_follows_contract() {
    let dir = tempfile::tempdir().unwrap();
    let base = spawn_server(dir.path().join("config.toml").to_str().unwrap()).await;
    let c = reqwest::Client::new();
    setup_installed(&c, &base, dir.path()).await;

    let r = c.get(format!("{base}/robots.txt")).send().await.unwrap();
    assert_eq!(r.status(), 200);
    let ct = r.headers().get("content-type").unwrap().to_str().unwrap();
    assert!(ct.starts_with("text/plain"), "{ct}");
    let body = r.text().await.unwrap();
    assert!(body.contains("User-agent: *"));
    assert!(body.contains("Allow: /"));
    assert!(body.contains("Disallow: /admin\n"), "{body}");
    assert!(body.contains("Disallow: /api/admin"));
    assert!(
        body.contains("Sitemap: https://blog.example.com/api/sitemap.xml"),
        "{body}"
    );
}

// ---------- 4. 页面 OG HTML（含停用 404 与转义） ----------

#[tokio::test(flavor = "multi_thread")]
async fn page_og_html_and_disabled_404() {
    let dir = tempfile::tempdir().unwrap();
    let base = spawn_server(dir.path().join("config.toml").to_str().unwrap()).await;
    let c = reqwest::Client::new();
    let token = setup_installed(&c, &base, dir.path()).await;

    let p = create_page(
        &c,
        &base,
        &token,
        json!({
            "title": "关于 <我们>",
            "slug": "about-us",
            "content_md": "# 标题\n\n这是一段**正文**说明。",
            "enabled": true
        }),
    )
    .await;

    let (status, html) = crawler_get(&c, format!("{base}/pages/about-us")).await;
    assert_eq!(status, 200);
    assert!(html.contains("<title>关于 &lt;我们&gt; - 测试博客</title>"));
    assert!(
        html.contains(r#"<link rel="canonical" href="https://blog.example.com/pages/about-us">"#)
    );
    assert!(html.contains(
        r#"<link rel="alternate" type="application/rss+xml" href="https://blog.example.com/api/feed.xml">"#
    ));
    assert!(html.contains(r#"<meta property="og:type" content="website">"#));
    // 页面无 excerpt：描述取正文纯文本截断（含正文文字、不含 Markdown 标记）
    assert!(
        html.contains(r#"content="标题 这是一段正文说明。""#),
        "{html}"
    );
    assert!(html.contains(r#""@type":"WebPage""#));

    // 停用 → 404
    let id = p["id"].as_i64().unwrap();
    let r = c
        .patch(format!("{base}/api/admin/pages/{id}/toggle"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    let (status, _, _) = crawler_get_raw(&c, format!("{base}/pages/about-us")).await;
    assert_eq!(status, 404, "停用页面应 404");

    // 停用页面即使普通 UA 也 302（不泄露存在性）
    let c_nr = no_redirect_client();
    let r = c_nr
        .get(format!("{base}/pages/about-us"))
        .header("user-agent", BROWSER_UA)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 302);
}

// ---------- 5. 未安装门禁 ----------

#[tokio::test(flavor = "multi_thread")]
async fn not_installed_html_routes_503() {
    let dir = tempfile::tempdir().unwrap();
    let base = spawn_server(dir.path().join("config.toml").to_str().unwrap()).await;
    let c = reqwest::Client::new();

    for path in ["/posts/whatever", "/pages/whatever", "/robots.txt"] {
        let r = c
            .get(format!("{base}{path}"))
            .header("user-agent", CRAWLER_UA)
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), 503, "{path} 未安装应 503");
        let v: Value = r.json().await.unwrap();
        assert_eq!(v["error"]["code"], "not_installed");
    }
}
