//! 站点设置集成测试（契约「站点设置」条款）：
//! - 未安装门禁：GET /api/site/settings 不在白名单 → 503
//! - 安装默认值：title/subtitle 取安装请求、per_page=10、其余为空；公开响应不含 base_url
//! - 鉴权：admin GET/PUT 未登录/无效 token → 401
//! - 更新回读：PUT 全量更新 → 公开 GET / admin GET / GET /api/site / 文章列表 per_page
//!   默认值 / feed.xml / sitemap.xml 全部实时联动（进程内生效，无需重启）
//! - 校验：per_page 越界、空标题、非法 base_url → 422 validation_error
//! - config.toml [server] base_url 作为设置项初始默认值

use reedblog_backend::state::AppState;
use serde_json::{json, Value};
use std::path::Path;

/// 在 127.0.0.1 随机端口起真实服务；预写 config.toml 把插件/主题/上传目录隔离到
/// tempdir；base_url 非 None 时写入 [server] base_url
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

// ---------- 1. 未安装门禁 ----------

#[tokio::test(flavor = "multi_thread")]
async fn site_settings_not_in_gate_whitelist() {
    let tmp = tempfile::tempdir().unwrap();
    let base = spawn_server(tmp.path().join("config.toml").to_str().unwrap(), None).await;
    let c = reqwest::Client::new();

    let r = c
        .get(format!("{base}/api/site/settings"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 503, "未安装时公开设置接口应 503");
    assert_eq!(err_code(r).await, "not_installed");
}

// ---------- 2. 安装默认值 + 公开响应不含敏感字段 ----------

#[tokio::test(flavor = "multi_thread")]
async fn defaults_after_install_and_public_shape() {
    let tmp = tempfile::tempdir().unwrap();
    let base = spawn_server(tmp.path().join("config.toml").to_str().unwrap(), None).await;
    let c = reqwest::Client::new();
    let token = setup_installed(&c, &base, tmp.path()).await;

    // 公开 GET：安装请求的 title/subtitle、per_page 默认 10、其余为空
    let r = c
        .get(format!("{base}/api/site/settings"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    let v: Value = r.json().await.unwrap();
    assert_eq!(
        v,
        json!({
            "title": "测试博客",
            "subtitle": "副标题",
            "description": "",
            "icp_number": "",
            "footer_text": "",
            "per_page": 10
        })
    );
    // 敏感字段 base_url 绝不出现在公开响应
    assert!(v.get("base_url").is_none(), "公开设置不应含 base_url");

    // 管理 GET：全部字段（含 base_url）
    let r = c
        .get(format!("{base}/api/admin/site/settings"))
        .header("Authorization", bearer(&token))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    let v: Value = r.json().await.unwrap();
    assert_eq!(v["title"], "测试博客");
    assert_eq!(v["per_page"], 10);
    assert_eq!(v["base_url"], "", "未配置 [server] base_url 时应为空串");
    assert!(v.get("base_url").is_some());

    // GET /api/site 与设置一致
    let r = c.get(format!("{base}/api/site")).send().await.unwrap();
    let v: Value = r.json().await.unwrap();
    assert_eq!(v["title"], "测试博客");
    assert_eq!(v["subtitle"], "副标题");
}

// ---------- 3. 管理接口鉴权 ----------

#[tokio::test(flavor = "multi_thread")]
async fn admin_settings_require_auth() {
    let tmp = tempfile::tempdir().unwrap();
    let base = spawn_server(tmp.path().join("config.toml").to_str().unwrap(), None).await;
    let c = reqwest::Client::new();
    let _token = setup_installed(&c, &base, tmp.path()).await;

    // 未带 token
    let r = c
        .get(format!("{base}/api/admin/site/settings"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 401);
    assert_eq!(err_code(r).await, "unauthorized");

    // 无效 token 的 PUT
    let r = c
        .put(format!("{base}/api/admin/site/settings"))
        .header("Authorization", "Bearer bad.token.here")
        .json(&json!({"title": "x", "per_page": 10}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 401);
    assert_eq!(err_code(r).await, "unauthorized");
}

// ---------- 4. 更新回读 + 全链路联动（进程内生效） ----------

#[tokio::test(flavor = "multi_thread")]
async fn update_settings_propagates_everywhere() {
    let tmp = tempfile::tempdir().unwrap();
    let base = spawn_server(tmp.path().join("config.toml").to_str().unwrap(), None).await;
    let c = reqwest::Client::new();
    let token = setup_installed(&c, &base, tmp.path()).await;

    // PUT 全量更新（base_url 带尾 /，应被规范化去掉）
    let body = json!({
        "title": " 新站名 ",
        "subtitle": "新口号",
        "description": "用于 meta description 的站点描述",
        "icp_number": "京ICP备12345678号",
        "footer_text": "本站由 reedblog 驱动",
        "per_page": 5,
        "base_url": "https://blog.example.com/"
    });
    let r = c
        .put(format!("{base}/api/admin/site/settings"))
        .header("Authorization", bearer(&token))
        .json(&body)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    let v: Value = r.json().await.unwrap();
    // 返回更新后的完整设置（含 base_url；title trim、base_url 去尾 /）
    assert_eq!(
        v,
        json!({
            "title": "新站名",
            "subtitle": "新口号",
            "description": "用于 meta description 的站点描述",
            "icp_number": "京ICP备12345678号",
            "footer_text": "本站由 reedblog 驱动",
            "per_page": 5,
            "base_url": "https://blog.example.com"
        })
    );

    // 公开 GET 实时联动（无重启）
    let r = c
        .get(format!("{base}/api/site/settings"))
        .send()
        .await
        .unwrap();
    let v: Value = r.json().await.unwrap();
    assert_eq!(v["title"], "新站名");
    assert_eq!(v["per_page"], 5);
    assert_eq!(v["icp_number"], "京ICP备12345678号");
    assert!(v.get("base_url").is_none());

    // GET /api/site 同步
    let r = c.get(format!("{base}/api/site")).send().await.unwrap();
    let v: Value = r.json().await.unwrap();
    assert_eq!(v["title"], "新站名");
    assert_eq!(v["subtitle"], "新口号");

    // 文章列表未传 per_page 时默认值 = 设置的 per_page（示例数据 3 篇）
    let r = c.get(format!("{base}/api/posts")).send().await.unwrap();
    let v: Value = r.json().await.unwrap();
    assert_eq!(v["per_page"], 5);
    assert_eq!(v["total"], 3);
    // 显式 per_page 仍优先
    let r = c
        .get(format!("{base}/api/posts?per_page=2"))
        .send()
        .await
        .unwrap();
    let v: Value = r.json().await.unwrap();
    assert_eq!(v["per_page"], 2);
    assert_eq!(v["items"].as_array().unwrap().len(), 2);

    // 搜索未传 per_page 同样取设置默认值
    let r = c
        .get(format!("{base}/api/search?q=reedblog"))
        .send()
        .await
        .unwrap();
    let v: Value = r.json().await.unwrap();
    assert_eq!(v["per_page"], 5);

    // feed.xml：channel title/description/link 读设置
    let r = c.get(format!("{base}/api/feed.xml")).send().await.unwrap();
    assert_eq!(r.status(), 200);
    let xml = r.text().await.unwrap();
    assert!(xml.contains("<title>新站名</title>"), "feed 标题应读设置");
    assert!(xml.contains("<description>新口号</description>"));
    assert!(
        xml.contains("<link>https://blog.example.com</link>"),
        "feed link 应优先用设置 base_url"
    );
    assert!(xml.contains("https://blog.example.com/posts/"));

    // sitemap.xml：绝对 URL 读设置 base_url
    let r = c
        .get(format!("{base}/api/sitemap.xml"))
        .send()
        .await
        .unwrap();
    let xml = r.text().await.unwrap();
    assert!(xml.contains("<loc>https://blog.example.com/</loc>"));
    assert!(xml.contains("<loc>https://blog.example.com/tags</loc>"));
}

// ---------- 5. 字段校验 → 422 ----------

#[tokio::test(flavor = "multi_thread")]
async fn update_settings_validation_errors() {
    let tmp = tempfile::tempdir().unwrap();
    let base = spawn_server(tmp.path().join("config.toml").to_str().unwrap(), None).await;
    let c = reqwest::Client::new();
    let token = setup_installed(&c, &base, tmp.path()).await;

    // 正例对照：合法 body 应 200（保证后续 422 都源于字段问题而非鉴权/请求形状问题；
    // 值与安装默认一致，不影响本测试末尾「校验失败不得改动已存设置」的断言）
    let valid = json!({"title": "测试博客", "per_page": 10});
    let r = c
        .put(format!("{base}/api/admin/site/settings"))
        .header("Authorization", bearer(&token))
        .json(&valid)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200, "合法 body 应更新成功");

    let bad_bodies = [
        json!({"title": "   ", "per_page": 10}), // 空标题
        json!({"title": "t", "per_page": 0}),    // per_page 下界
        json!({"title": "t", "per_page": 101}),  // per_page 上界
        json!({"title": "t", "per_page": 10, "base_url": "not a url"}),
        json!({"title": "t", "per_page": 10, "base_url": "ftp://x.dev"}),
        json!({"title": "t", "per_page": 10, "base_url": "blog.example.com"}), // 缺 scheme
        json!({"title": "长".repeat(256), "per_page": 10}),                    // 标题超长
    ];
    for body in bad_bodies {
        let r = c
            .put(format!("{base}/api/admin/site/settings"))
            .header("Authorization", bearer(&token))
            .json(&body)
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), 422, "应 422: {body}");
        assert_eq!(err_code(r).await, "validation_error", "{body}");
    }

    // 校验失败的 PUT 不得改动已存设置
    let r = c
        .get(format!("{base}/api/site/settings"))
        .send()
        .await
        .unwrap();
    let v: Value = r.json().await.unwrap();
    assert_eq!(v["title"], "测试博客");
    assert_eq!(v["per_page"], 10);
}

// ---------- 6. config.toml [server] base_url 作为初始默认 ----------

#[tokio::test(flavor = "multi_thread")]
async fn config_base_url_seeds_setting_default() {
    let tmp = tempfile::tempdir().unwrap();
    let base = spawn_server(
        tmp.path().join("config.toml").to_str().unwrap(),
        Some("https://configured.example.com"),
    )
    .await;
    let c = reqwest::Client::new();
    let token = setup_installed(&c, &base, tmp.path()).await;

    // 安装后 admin GET 的 base_url 初始值 = config.toml [server] base_url
    let r = c
        .get(format!("{base}/api/admin/site/settings"))
        .header("Authorization", bearer(&token))
        .send()
        .await
        .unwrap();
    let v: Value = r.json().await.unwrap();
    assert_eq!(v["base_url"], "https://configured.example.com");

    // feed 链接即用该初始默认
    let r = c.get(format!("{base}/api/feed.xml")).send().await.unwrap();
    let xml = r.text().await.unwrap();
    assert!(xml.contains("<link>https://configured.example.com</link>"));

    // 设置里清空 base_url 后回退 config.toml 值（load 按键回退：空串仍视为「已设置」，
    // 显式存空串时 feed 走 config → 请求头推导链，此处 config 非空应兜底）
    let r = c
        .put(format!("{base}/api/admin/site/settings"))
        .header("Authorization", bearer(&token))
        .json(&json!({"title": "测试博客", "per_page": 10, "base_url": ""}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    let r = c.get(format!("{base}/api/feed.xml")).send().await.unwrap();
    let xml = r.text().await.unwrap();
    assert!(
        xml.contains("<link>https://configured.example.com</link>"),
        "设置 base_url 为空时应回退 config.toml 值"
    );
}
