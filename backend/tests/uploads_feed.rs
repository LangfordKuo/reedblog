//! 图片上传 + RSS feed + sitemap 集成测试（契约 2026-10-03 新增条款）：
//! - uploads：未登录 401；png 上传成功（sha256 命名/落盘）；静态读取 Content-Type 与强缓存；
//!   伪造扩展名/svg → 422 invalid_file_type；超大 → 422 file_too_large；同内容去重；目录穿越防护
//! - feed.xml：RSS 2.0、仅 published、XML 转义、RFC 822 pubDate、20 篇上限、
//!   X-Forwarded-* 推导 base、[server] base_url 优先
//! - sitemap.xml：首页/文章（lastmod）/标签/分类/归档

use reedblog_backend::state::AppState;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::path::Path;

/// 在 127.0.0.1 随机端口起真实服务；预写 config.toml 把插件/主题/上传目录隔离到
/// tempdir（测试隔离，避免写入仓库工作目录）。uploads_max_mb 覆盖 [uploads] max_size_mb，
/// base_url 写入 [server] base_url（None 则不写该键）。
async fn spawn_server(
    config_path: &str,
    uploads_max_mb: Option<u64>,
    base_url: Option<&str>,
) -> String {
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
        if let Some(mb) = uploads_max_mb {
            text.push_str(&format!("max_size_mb = {mb}\n"));
        }
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

/// 安装（站点标题「测试博客」/副标题「副标题」）+ 登录，返回 Bearer token
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

/// multipart 上传图片（字段名 file；token=None 模拟未登录）
async fn upload_image(
    c: &reqwest::Client,
    base: &str,
    token: Option<&str>,
    filename: &str,
    data: Vec<u8>,
) -> reqwest::Response {
    let part = reqwest::multipart::Part::bytes(data).file_name(filename.to_string());
    let form = reqwest::multipart::Form::new().part("file", part);
    let mut req = c.post(format!("{base}/api/admin/uploads")).multipart(form);
    if let Some(t) = token {
        req = req.bearer_auth(t);
    }
    req.send().await.unwrap()
}

async fn create_post(c: &reqwest::Client, base: &str, token: &str, body: Value) -> Value {
    let r = c
        .post(format!("{base}/api/admin/posts"))
        .bearer_auth(token)
        .json(&body)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 201, "{}", r.text().await.unwrap());
    r.json().await.unwrap()
}

/// 构造 total_len 字节的伪 PNG（真实 magic 头 + 零填充）
fn png_bytes(total_len: usize) -> Vec<u8> {
    let mut v = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
    v.resize(total_len.max(8), 0u8);
    v
}

fn sha256_hex(data: &[u8]) -> String {
    Sha256::digest(data)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

// ---------- 1. 图片上传：鉴权/成功链路/类型与大小校验/去重/穿越 ----------

#[tokio::test(flavor = "multi_thread")]
async fn image_upload_flow() {
    let tmp = tempfile::tempdir().unwrap();
    let base = spawn_server(
        tmp.path().join("config.toml").to_str().unwrap(),
        Some(1), // max_size_mb = 1，方便用小文件触发 file_too_large
        None,
    )
    .await;
    let c = reqwest::Client::new();

    // 未安装门禁：uploads/feed/sitemap 都不在白名单 → 503 not_installed
    for p in ["/api/uploads/x.png", "/api/feed.xml", "/api/sitemap.xml"] {
        let r = c.get(format!("{base}{p}")).send().await.unwrap();
        assert_eq!(r.status(), 503, "GET {p} 未安装时应 503");
        assert_eq!(err_code(r).await, "not_installed", "GET {p}");
    }
    let r = upload_image(&c, &base, None, "a.png", png_bytes(16)).await;
    assert_eq!(r.status(), 503);
    assert_eq!(err_code(r).await, "not_installed");

    let token = setup_installed(&c, &base, tmp.path()).await;

    // 已安装但未登录 → 401 unauthorized
    let png = png_bytes(64);
    let r = upload_image(&c, &base, None, "a.png", png.clone()).await;
    assert_eq!(r.status(), 401);
    assert_eq!(err_code(r).await, "unauthorized");

    // 上传 png 成功 → 200 UploadResult
    let r = upload_image(&c, &base, Some(&token), "my photo.png", png.clone()).await;
    assert_eq!(r.status(), 200, "{}", r.text().await.unwrap());
    let v: Value = r.json().await.unwrap();
    let url = v["url"].as_str().unwrap().to_string();
    assert!(url.starts_with("/api/uploads/"), "{url}");
    assert_eq!(v["size"], png.len() as u64);
    assert_eq!(v["filename"], "my photo.png");

    // 文件名 = <yyyy>/<mm>/<sha256 前 16 位 hex>.png（不含用户输入）
    let hex = sha256_hex(&png);
    let hash16 = &hex[..16];
    let ym = chrono::Utc::now().format("%Y/%m").to_string();
    assert_eq!(url, format!("/api/uploads/{ym}/{hash16}.png"));
    // 已落盘到隔离的 uploads 目录
    assert!(tmp
        .path()
        .join("uploads")
        .join(&ym)
        .join(format!("{hash16}.png"))
        .is_file());

    // GET 静态读取 → 200 + 正确 Content-Type + 强缓存 + 字节一致
    let r = c.get(format!("{base}{url}")).send().await.unwrap();
    assert_eq!(r.status(), 200);
    assert_eq!(
        r.headers().get("content-type").unwrap().to_str().unwrap(),
        "image/png"
    );
    let cc = r.headers().get("cache-control").unwrap().to_str().unwrap();
    assert!(cc.contains("public"), "{cc}");
    assert!(cc.contains("max-age=31536000"), "{cc}");
    assert!(cc.contains("immutable"), "{cc}");
    assert_eq!(r.bytes().await.unwrap().as_ref(), png.as_slice());

    // 同内容重复上传 → 同一 url（内容哈希去重，不重复落盘）；filename 仅回显
    let r = upload_image(&c, &base, Some(&token), "renamed.png", png.clone()).await;
    assert_eq!(r.status(), 200);
    let v2: Value = r.json().await.unwrap();
    assert_eq!(v2["url"].as_str().unwrap(), url);
    assert_eq!(v2["filename"], "renamed.png");

    // 伪造扩展名：内容非图片 → 422 invalid_file_type
    let r = upload_image(
        &c,
        &base,
        Some(&token),
        "evil.png",
        b"definitely not an image".to_vec(),
    )
    .await;
    assert_eq!(r.status(), 422);
    assert_eq!(err_code(r).await, "invalid_file_type");

    // svg 一律拒绝（XSS 风险），即使扩展名/内容「名副其实」
    let svg = b"<svg xmlns='http://www.w3.org/2000/svg'><script>alert(1)</script></svg>".to_vec();
    let r = upload_image(&c, &base, Some(&token), "xss.svg", svg).await;
    assert_eq!(r.status(), 422);
    assert_eq!(err_code(r).await, "invalid_file_type");

    // jpeg：真实类型决定扩展名与 Content-Type（故意用错误扩展名 .gif 上传）
    let mut jpeg = vec![0xff, 0xd8, 0xff, 0xe1];
    jpeg.resize(32, 7);
    let r = upload_image(&c, &base, Some(&token), "photo.gif", jpeg.clone()).await;
    assert_eq!(r.status(), 200);
    let vj: Value = r.json().await.unwrap();
    let jpeg_url = vj["url"].as_str().unwrap();
    assert!(jpeg_url.ends_with(".jpg"), "{jpeg_url}");
    let r = c.get(format!("{base}{jpeg_url}")).send().await.unwrap();
    assert_eq!(
        r.headers().get("content-type").unwrap().to_str().unwrap(),
        "image/jpeg"
    );

    // gif / webp magic 均放行
    let mut gif = b"GIF89a".to_vec();
    gif.resize(32, 0);
    let r = upload_image(&c, &base, Some(&token), "anim.gif", gif).await;
    assert_eq!(r.status(), 200);
    assert!(r.json::<Value>().await.unwrap()["url"]
        .as_str()
        .unwrap()
        .ends_with(".gif"));

    let mut webp = b"RIFF".to_vec();
    webp.extend_from_slice(&[0x14, 0, 0, 0]);
    webp.extend_from_slice(b"WEBPVP8 ");
    webp.resize(40, 0);
    let r = upload_image(&c, &base, Some(&token), "img.png", webp).await;
    assert_eq!(r.status(), 200);
    assert!(r.json::<Value>().await.unwrap()["url"]
        .as_str()
        .unwrap()
        .ends_with(".webp"));

    // 超大文件 → 422 file_too_large（测试配置 max_size_mb=1）
    let big = png_bytes(1024 * 1024 + 16);
    let r = upload_image(&c, &base, Some(&token), "big.png", big).await;
    assert_eq!(r.status(), 422);
    assert_eq!(err_code(r).await, "file_too_large");

    // 缺 file 字段 → 422
    let form = reqwest::multipart::Form::new().text("other", "x".to_string());
    let r = c
        .post(format!("{base}/api/admin/uploads"))
        .bearer_auth(&token)
        .multipart(form)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 422);

    // 目录穿越 → 404/400，绝不泄漏 config.toml 内容
    for attack in [
        "/api/uploads/../config.toml",
        "/api/uploads/..%2Fconfig.toml",
        "/api/uploads/%2e%2e/config.toml",
        "/api/uploads/..%2F..%2F..%2Fconfig.toml",
        "/api/uploads/..%5Cconfig.toml",
        "/api/uploads/%2e%2e%2f%2e%2e%2fconfig.toml",
    ] {
        let r = c.get(format!("{base}{attack}")).send().await.unwrap();
        assert_ne!(r.status(), 200, "目录穿越不应成功: {attack}");
        assert!(
            r.status() == 404 || r.status() == 400,
            "穿越应 404/400: {attack} → {}",
            r.status()
        );
        let text = r.text().await.unwrap();
        assert!(
            !text.contains("jwt_secret"),
            "不得泄漏 config.toml: {attack}"
        );
    }
    // 合法路径但文件不存在 → 404
    let r = c
        .get(format!("{base}/api/uploads/2026/01/no-such-file.png"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 404);
}

// ---------- 2. feed.xml + sitemap.xml ----------

#[tokio::test(flavor = "multi_thread")]
async fn feed_and_sitemap() {
    let tmp = tempfile::tempdir().unwrap();
    let base = spawn_server(tmp.path().join("config.toml").to_str().unwrap(), None, None).await;
    let c = reqwest::Client::new();
    let token = setup_installed(&c, &base, tmp.path()).await;

    create_post(
        &c,
        &base,
        &token,
        json!({
            "title": "Feed & <RSS> 特殊字符", "slug": "feed-esc",
            "excerpt": "摘要 & <escape>", "content_md": "正文A", "status": "published"
        }),
    )
    .await;
    create_post(
        &c,
        &base,
        &token,
        json!({
            "title": "Feed 测试文章", "slug": "feed-post",
            "content_md": "# 正文", "status": "published"
        }),
    )
    .await;
    create_post(
        &c,
        &base,
        &token,
        json!({"title": "秘密草稿", "slug": "draft-x", "content_md": "x", "status": "draft"}),
    )
    .await;

    // ---- feed.xml ----
    let r = c.get(format!("{base}/api/feed.xml")).send().await.unwrap();
    assert_eq!(r.status(), 200);
    let ct = r.headers().get("content-type").unwrap().to_str().unwrap();
    assert!(ct.starts_with("application/rss+xml"), "{ct}");
    assert!(ct.contains("charset=utf-8"), "{ct}");
    let xml = r.text().await.unwrap();
    assert!(
        xml.starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\"?>"),
        "{xml}"
    );
    assert!(xml.contains("<rss version=\"2.0\">"), "{xml}");
    // channel：站点标题 + 副标题
    assert!(xml.contains("<title>测试博客</title>"), "{xml}");
    assert!(xml.contains("<description>副标题</description>"), "{xml}");
    // 所有文本 XML 转义
    assert!(
        xml.contains("<title>Feed &amp; &lt;RSS&gt; 特殊字符</title>"),
        "{xml}"
    );
    assert!(xml.contains("摘要 &amp; &lt;escape&gt;"), "{xml}");
    // excerpt 缺省 → 从正文推导（无 Markdown 符号）
    assert!(xml.contains("<description>正文</description>"), "{xml}");
    // link/guid = 文章前台绝对 URL（与前端路由 /posts/:slug 一致；Host 头推导）
    assert!(
        xml.contains(&format!("<link>{base}/posts/feed-post</link>")),
        "{xml}"
    );
    assert!(
        xml.contains(&format!(
            "<guid isPermaLink=\"true\">{base}/posts/feed-esc</guid>"
        )),
        "{xml}"
    );
    // pubDate 为 RFC 822（UTC 时区 +0000）
    assert!(xml.contains("<pubDate>"), "{xml}");
    assert!(xml.contains("+0000</pubDate>"), "{xml}");
    // 草稿不出现；item 数 = published 数（2 篇测试文章 + 3 篇安装注入的示例文章）
    assert!(!xml.contains("秘密草稿"), "{xml}");
    assert!(!xml.contains("draft-x"), "{xml}");
    assert_eq!(xml.matches("<item>").count(), 5, "{xml}");

    // 反代场景：X-Forwarded-Proto/Host 决定绝对 URL
    let r = c
        .get(format!("{base}/api/feed.xml"))
        .header("x-forwarded-proto", "https")
        .header("x-forwarded-host", "blog.example.com, proxy-internal")
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    let xml2 = r.text().await.unwrap();
    assert!(
        xml2.contains("<link>https://blog.example.com</link>"),
        "{xml2}"
    );
    assert!(
        xml2.contains("https://blog.example.com/posts/feed-post"),
        "{xml2}"
    );

    // ---- sitemap.xml ----
    let r = c
        .get(format!("{base}/api/sitemap.xml"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    let ct = r.headers().get("content-type").unwrap().to_str().unwrap();
    assert!(ct.starts_with("application/xml"), "{ct}");
    assert!(ct.contains("charset=utf-8"), "{ct}");
    let sm = r.text().await.unwrap();
    assert!(
        sm.contains("<urlset xmlns=\"http://www.sitemaps.org/schemas/sitemap/0.9\">"),
        "{sm}"
    );
    // 首页 / 文章 / 标签索引 / 分类索引 / 归档（前端真实路由）
    assert!(sm.contains(&format!("<loc>{base}/</loc>")), "{sm}");
    assert!(
        sm.contains(&format!("<loc>{base}/posts/feed-post</loc>")),
        "{sm}"
    );
    assert!(
        sm.contains(&format!("<loc>{base}/posts/feed-esc</loc>")),
        "{sm}"
    );
    assert!(sm.contains(&format!("<loc>{base}/tags</loc>")), "{sm}");
    assert!(
        sm.contains(&format!("<loc>{base}/categories</loc>")),
        "{sm}"
    );
    assert!(sm.contains(&format!("<loc>{base}/archive</loc>")), "{sm}");
    // 草稿不出现
    assert!(!sm.contains("draft-x"), "{sm}");
    // lastmod = updated_at（W3C datetime，即库中 RFC3339 格式）
    let lastmod = sm
        .split("<lastmod>")
        .nth(1)
        .and_then(|s| s.split("</lastmod>").next())
        .expect("文章条目应含 lastmod");
    assert!(
        chrono::DateTime::parse_from_rfc3339(lastmod).is_ok(),
        "lastmod 应为 W3C datetime: {lastmod}"
    );
}

// ---------- 3. feed 20 篇上限 ----------

#[tokio::test(flavor = "multi_thread")]
async fn feed_limits_to_20_items() {
    let tmp = tempfile::tempdir().unwrap();
    let base = spawn_server(tmp.path().join("config.toml").to_str().unwrap(), None, None).await;
    let c = reqwest::Client::new();
    let token = setup_installed(&c, &base, tmp.path()).await;

    for i in 0..21 {
        create_post(
            &c,
            &base,
            &token,
            json!({
                "title": format!("Cap Post {i:02}"), "slug": format!("cap-{i:02}"),
                "content_md": "x", "status": "published"
            }),
        )
        .await;
    }
    let xml = c
        .get(format!("{base}/api/feed.xml"))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert_eq!(xml.matches("<item>").count(), 20, "feed 只输出最新 20 篇");
    // sitemap 不受 20 篇限制：21 篇测试文章 + 3 篇安装注入的示例文章全部收录
    let sm = c
        .get(format!("{base}/api/sitemap.xml"))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert_eq!(sm.matches("<url>").count(), 24 + 4, "{sm}"); // 24 文章 + 首页/tags/categories/archive
}

// ---------- 4. [server] base_url 优先于请求头 ----------

#[tokio::test(flavor = "multi_thread")]
async fn base_url_config_overrides_headers() {
    let tmp = tempfile::tempdir().unwrap();
    // 带尾斜杠，验证去尾
    let base = spawn_server(
        tmp.path().join("config.toml").to_str().unwrap(),
        None,
        Some("https://configured.example.org/"),
    )
    .await;
    let c = reqwest::Client::new();
    let token = setup_installed(&c, &base, tmp.path()).await;
    create_post(
        &c,
        &base,
        &token,
        json!({"title": "Config URL 测试", "slug": "cfg-post", "content_md": "x", "status": "published"}),
    )
    .await;

    // 即使带了 X-Forwarded-*，也以 config base_url 为准
    let xml = c
        .get(format!("{base}/api/feed.xml"))
        .header("x-forwarded-proto", "http")
        .header("x-forwarded-host", "other.example.com")
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(
        xml.contains("<link>https://configured.example.org</link>"),
        "{xml}"
    );
    assert!(
        xml.contains("https://configured.example.org/posts/cfg-post"),
        "{xml}"
    );

    let sm = c
        .get(format!("{base}/api/sitemap.xml"))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(
        sm.contains("<loc>https://configured.example.org/</loc>"),
        "{sm}"
    );
    assert!(
        sm.contains("<loc>https://configured.example.org/posts/cfg-post</loc>"),
        "{sm}"
    );
    assert!(
        sm.contains("<loc>https://configured.example.org/tags</loc>"),
        "{sm}"
    );
}
