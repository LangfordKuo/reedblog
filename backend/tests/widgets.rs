//! 主题组件（widgets）集成测试（api-contract.md「主题组件」+ 扩展契约「主题组件」）：
//! - 默认值：default 主题默认启用 recent-posts/tag-cloud/categories（sidebar），
//!   公开端点仅返回 enabled 组件，管理端点返回全量合并列表 + positions
//! - GET/PUT 回读：全量替换语义、参数规范化、公开端点即时反映、
//!   custom 组件 {{param}} 令牌替换与 html 覆盖
//! - 按主题隔离：切换主题各自配置不丢；删除主题连带清理（重装回默认值）
//! - 校验：非法 position / 未注册 key → 422（invalid_value / unknown_widget）、
//!   重复 key、非法 custom key、html 超长、声明非法的 zip → 422 invalid_manifest
//! - 自定义组件增删：PUT 全量列表中加入/移除 custom 行
//! - 未安装门禁白名单：GET /api/themes/default/widgets 可用（内置默认值），管理端点 503
//! - 配套公开接口：GET /api/site/stats、GET /api/posts?order=hot

use serde_json::{json, Value};
use std::io::Write;
use std::path::Path;

/// 在 127.0.0.1 随机端口起真实服务；预写 config.toml 把插件/主题目录隔离到 tempdir
async fn spawn_server(config_path: &str) -> String {
    if !Path::new(config_path).exists() {
        let dir = Path::new(config_path).parent().unwrap();
        let toml_path = |p: std::path::PathBuf| p.to_str().unwrap().replace('\\', "/");
        std::fs::write(
            config_path,
            format!(
                "[plugins]\ndir = \"{}\"\n\n[themes]\ndir = \"{}\"\n",
                toml_path(dir.join("plugins")),
                toml_path(dir.join("themes")),
            ),
        )
        .unwrap();
    }
    let state = reedblog_backend::state::AppState::new(config_path);
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

fn install_body(dir: &Path) -> Value {
    json!({
        "db_type": "sqlite",
        "sqlite_path": dir.join("reedblog.db").to_str().unwrap(),
        "admin": {"username": "admin", "password": "secret123"},
        "site": {"title": "组件测试博客"}
    })
}

/// 安装 + 登录，返回 Bearer token
async fn setup_installed(c: &reqwest::Client, base: &str, dir: &Path) -> String {
    let r = c
        .post(format!("{base}/api/install"))
        .json(&install_body(dir))
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

/// 内存构造 zip（条目名 → 内容）
fn build_zip(files: &[(&str, &str)]) -> Vec<u8> {
    let mut zw = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let opts = zip::write::SimpleFileOptions::default();
    for (name, content) in files {
        zw.start_file(*name, opts).unwrap();
        zw.write_all(content.as_bytes()).unwrap();
    }
    zw.finish().unwrap().into_inner()
}

async fn upload(c: &reqwest::Client, url: &str, token: &str, zip: Vec<u8>) -> reqwest::Response {
    let part = reqwest::multipart::Part::bytes(zip)
        .file_name("pkg.zip")
        .mime_str("application/zip")
        .unwrap();
    let form = reqwest::multipart::Form::new().part("file", part);
    c.post(url)
        .bearer_auth(token)
        .multipart(form)
        .send()
        .await
        .unwrap()
}

/// GET 公开生效组件配置
async fn public_widgets(c: &reqwest::Client, base: &str, slug: &str) -> (u16, Value) {
    let r = c
        .get(format!("{base}/api/themes/{slug}/widgets"))
        .send()
        .await
        .unwrap();
    let status = r.status().as_u16();
    (
        status,
        if status == 200 {
            r.json().await.unwrap()
        } else {
            Value::Null
        },
    )
}

/// GET 管理全量组件配置
async fn admin_widgets(c: &reqwest::Client, base: &str, token: &str, slug: &str) -> (u16, Value) {
    let r = c
        .get(format!("{base}/api/admin/themes/{slug}/widgets"))
        .bearer_auth(token)
        .send()
        .await
        .unwrap();
    let status = r.status().as_u16();
    (
        status,
        if status == 200 {
            r.json().await.unwrap()
        } else {
            Value::Null
        },
    )
}

/// PUT 全量替换组件配置
async fn put_widgets(
    c: &reqwest::Client,
    base: &str,
    token: &str,
    slug: &str,
    widgets: Value,
) -> (u16, Value) {
    let r = c
        .put(format!("{base}/api/admin/themes/{slug}/widgets"))
        .bearer_auth(token)
        .json(&json!({ "widgets": widgets }))
        .send()
        .await
        .unwrap();
    let status = r.status().as_u16();
    (status, r.json().await.unwrap())
}

fn widget_keys(v: &Value) -> Vec<String> {
    v["widgets"]
        .as_array()
        .unwrap()
        .iter()
        .map(|w| w["key"].as_str().unwrap().to_string())
        .collect()
}

fn find<'a>(v: &'a Value, key: &str) -> &'a Value {
    v["widgets"]
        .as_array()
        .unwrap()
        .iter()
        .find(|w| w["key"] == key)
        .unwrap_or(&Value::Null)
}

/// 带 [[widgets]] 声明与 HTML 片段文件的第三方主题
const WIDGET_THEME_TOML: &str = r##"name = "组件主题"
slug = "widget-theme"
version = "1.0.0"

[tokens]
background = "0 0% 100%"

[[widgets]]
key = "notice"
label = "公告栏"
default_enabled = true
default_position = "footer"
default_sort = 70

[[widgets.params]]
key = "text"
label = "公告文字"
type = "text"
default = "欢迎"
"##;

fn widget_theme_zip() -> Vec<u8> {
    build_zip(&[
        ("widget-theme/theme.toml", WIDGET_THEME_TOML),
        (
            "widget-theme/assets/widgets/notice.html",
            "<p class=\"notice\">{{text}}</p>",
        ),
    ])
}

// ---------- 1. 默认值 + 公开/管理端点形状 ----------

#[tokio::test(flavor = "multi_thread")]
async fn defaults_public_and_admin_shapes() {
    let tmp = tempfile::tempdir().unwrap();
    let base = spawn_server(tmp.path().join("config.toml").to_str().unwrap()).await;
    let c = reqwest::Client::new();
    let token = setup_installed(&c, &base, tmp.path()).await;

    // 公开端点：仅默认启用集（recent-posts/tag-cloud/categories），sort_order ASC
    let (status, v) = public_widgets(&c, &base, "default").await;
    assert_eq!(status, 200);
    assert_eq!(v["slug"], "default");
    assert_eq!(
        widget_keys(&v),
        vec!["recent-posts", "tag-cloud", "categories"]
    );
    let rp = find(&v, "recent-posts");
    assert_eq!(rp["kind"], "builtin");
    assert_eq!(rp["position"], "sidebar");
    assert_eq!(rp["sort_order"], 10);
    assert_eq!(rp["config"]["title"], "最新文章");
    assert_eq!(rp["config"]["count"], 5);
    // 公开形状不带管理字段
    assert!(rp.get("source").is_none() && rp.get("params").is_none());

    // 管理端点：全量 7 个内置组件 + positions 规范枚举 + params 声明
    let (status, a) = admin_widgets(&c, &base, &token, "default").await;
    assert_eq!(status, 200);
    assert_eq!(a["slug"], "default");
    assert_eq!(
        a["positions"],
        json!(["sidebar", "left", "right", "footer"])
    );
    assert_eq!(a["widgets"].as_array().unwrap().len(), 7);
    let enabled: Vec<&str> = a["widgets"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|w| w["enabled"] == json!(true))
        .map(|w| w["key"].as_str().unwrap())
        .collect();
    assert_eq!(enabled, vec!["recent-posts", "tag-cloud", "categories"]);
    let hot = find(&a, "hot-posts");
    assert_eq!(hot["source"], "builtin");
    assert_eq!(hot["enabled"], false);
    assert!(hot["params"]
        .as_array()
        .unwrap()
        .iter()
        .any(|p| p["key"] == "count" && p["default"] == json!(5)));

    // 无 token → 401
    let r = c
        .get(format!("{base}/api/admin/themes/default/widgets"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 401);
}

// ---------- 2. PUT 全量替换 + 回读 + custom 组件增删 + 令牌替换 ----------

#[tokio::test(flavor = "multi_thread")]
async fn put_readback_and_custom_lifecycle() {
    let tmp = tempfile::tempdir().unwrap();
    let base = spawn_server(tmp.path().join("config.toml").to_str().unwrap()).await;
    let c = reqwest::Client::new();
    let token = setup_installed(&c, &base, tmp.path()).await;

    // 全量配置：启用 hot-posts（右栏、count=3）、停用 tag-cloud、新建 custom-box（页脚）
    let (status, saved) = put_widgets(
        &c,
        &base,
        &token,
        "default",
        json!([
            {"key": "recent-posts", "kind": "builtin", "enabled": true,
             "position": "sidebar", "sort_order": 10, "config": {"title": "最新文章", "count": 5}},
            {"key": "hot-posts", "kind": "builtin", "enabled": true,
             "position": "right", "sort_order": 15, "config": {"title": "热门文章", "count": 3}},
            {"key": "tag-cloud", "kind": "builtin", "enabled": false,
             "position": "sidebar", "sort_order": 30},
            {"key": "categories", "kind": "builtin", "enabled": true,
             "position": "sidebar", "sort_order": 40},
            {"key": "custom-box", "kind": "custom", "enabled": true,
             "position": "footer", "sort_order": 100,
             "config": {"title": "盒子", "html": "<b>{{title}}</b>"}},
        ]),
    )
    .await;
    assert_eq!(status, 200, "{saved}");
    // 管理响应 = 全量合并（7 内置 + 1 自建）
    assert_eq!(saved["widgets"].as_array().unwrap().len(), 8);
    let box_ = find(&saved, "custom-box");
    assert_eq!(box_["source"], "admin");
    assert_eq!(box_["kind"], "custom");
    assert_eq!(box_["label"], "盒子"); // label 取 config.title
    assert_eq!(box_["config"]["html"], "<b>{{title}}</b>"); // 管理端为原始值（未替换）

    // 公开端点：仅 enabled，按 sort_order ASC；custom html 已做令牌替换
    let (_, v) = public_widgets(&c, &base, "default").await;
    assert_eq!(
        widget_keys(&v),
        vec!["recent-posts", "hot-posts", "categories", "custom-box"]
    );
    let hot = find(&v, "hot-posts");
    assert_eq!(hot["position"], "right");
    assert_eq!(hot["config"]["count"], 3);
    assert_eq!(find(&v, "custom-box")["config"]["html"], "<b>盒子</b>");

    // 回读一致性：管理 GET 与 PUT 响应相同
    let (_, a) = admin_widgets(&c, &base, &token, "default").await;
    assert_eq!(a, saved);

    // 全量替换语义：提交不含 custom-box / hot-posts / tag-cloud 覆盖的列表 →
    // 自建组件被删除；未出现的内置组件回注册表默认值
    // （hot-posts 默认停用；tag-cloud 此前被停用、现在回默认启用）
    let (status, saved2) = put_widgets(
        &c,
        &base,
        &token,
        "default",
        json!([
            {"key": "recent-posts", "kind": "builtin", "enabled": true,
             "position": "sidebar", "sort_order": 10},
            {"key": "archive", "kind": "builtin", "enabled": true,
             "position": "left", "sort_order": 20},
        ]),
    )
    .await;
    assert_eq!(status, 200);
    assert!(find(&saved2, "custom-box").is_null(), "自建组件应被删除");
    assert_eq!(find(&saved2, "hot-posts")["enabled"], false);
    assert_eq!(find(&saved2, "hot-posts")["position"], "sidebar");
    assert_eq!(
        find(&saved2, "tag-cloud")["enabled"],
        true,
        "未出现的内置组件应回默认值（tag-cloud 默认启用）"
    );
    let (_, v2) = public_widgets(&c, &base, "default").await;
    assert_eq!(
        widget_keys(&v2),
        vec!["recent-posts", "archive", "tag-cloud", "categories"]
    );
}

// ---------- 3. 校验：非法 position/key、重复、超长、声明非法 ----------

#[tokio::test(flavor = "multi_thread")]
async fn validation_errors() {
    let tmp = tempfile::tempdir().unwrap();
    let base = spawn_server(tmp.path().join("config.toml").to_str().unwrap()).await;
    let c = reqwest::Client::new();
    let token = setup_installed(&c, &base, tmp.path()).await;

    // 非法 position → 422 invalid_value
    let (status, body) = put_widgets(
        &c,
        &base,
        &token,
        "default",
        json!([{"key": "archive", "kind": "builtin", "position": "header"}]),
    )
    .await;
    assert_eq!(status, 422);
    assert_eq!(body["error"]["code"], "invalid_value");

    // 未注册的 builtin key → 422 unknown_widget
    let (status, body) = put_widgets(
        &c,
        &base,
        &token,
        "default",
        json!([{"key": "no-such-widget", "kind": "builtin", "position": "sidebar"}]),
    )
    .await;
    assert_eq!(status, 422);
    assert_eq!(body["error"]["code"], "unknown_widget");

    // key 重复 → 422 invalid_value
    let (status, body) = put_widgets(
        &c,
        &base,
        &token,
        "default",
        json!([
            {"key": "archive", "kind": "builtin", "position": "sidebar"},
            {"key": "archive", "kind": "builtin", "position": "left"},
        ]),
    )
    .await;
    assert_eq!(status, 422);
    assert_eq!(body["error"]["code"], "invalid_value");

    // custom key 与内置冲突 / key 非法 → 422 invalid_value
    let (status, body) = put_widgets(
        &c,
        &base,
        &token,
        "default",
        json!([{"key": "archive", "kind": "custom", "position": "sidebar"}]),
    )
    .await;
    assert_eq!(status, 422);
    assert_eq!(body["error"]["code"], "invalid_value");
    let (status, _) = put_widgets(
        &c,
        &base,
        &token,
        "default",
        json!([{"key": "Bad Key", "kind": "custom", "position": "sidebar"}]),
    )
    .await;
    assert_eq!(status, 422);

    // config 含未声明参数 → 422 invalid_value；html 超长 → 422
    let (status, body) = put_widgets(
        &c,
        &base,
        &token,
        "default",
        json!([{"key": "recent-posts", "kind": "builtin", "position": "sidebar",
                "config": {"nope": 1}}]),
    )
    .await;
    assert_eq!(status, 422);
    assert_eq!(body["error"]["code"], "invalid_value");
    let (status, _) = put_widgets(
        &c,
        &base,
        &token,
        "default",
        json!([{"key": "custom-x", "kind": "custom", "position": "sidebar",
                "config": {"html": "长".repeat(65537)}}]),
    )
    .await;
    assert_eq!(status, 422);

    // widgets 非数组 → 422 validation_error
    let r = c
        .put(format!("{base}/api/admin/themes/default/widgets"))
        .bearer_auth(&token)
        .json(&json!({ "widgets": {} }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 422);
    assert_eq!(err_code(r).await, "validation_error");

    // slug 未安装 → 404；无 token → 401
    let (status, _) = put_widgets(&c, &base, &token, "nope", json!([])).await;
    assert_eq!(status, 404);
    let r = c
        .put(format!("{base}/api/admin/themes/default/widgets"))
        .json(&json!({ "widgets": [] }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 401);

    // 校验失败不写库：此前的配置保持默认（公开端点仍是默认启用集）
    let (_, v) = public_widgets(&c, &base, "default").await;
    assert_eq!(
        widget_keys(&v),
        vec!["recent-posts", "tag-cloud", "categories"]
    );

    // [[widgets]] 声明非法的 zip → 上传 422 invalid_manifest
    let bad_position = r#"name = "坏组件"
slug = "bad-widget"
version = "1.0.0"

[[widgets]]
key = "notice"
default_position = "header"
"#;
    let r = upload(
        &c,
        &format!("{base}/api/admin/themes"),
        &token,
        build_zip(&[("bad-widget/theme.toml", bad_position)]),
    )
    .await;
    assert_eq!(r.status(), 422);
    assert_eq!(err_code(r).await, "invalid_manifest");

    let builtin_conflict = r#"name = "坏组件"
slug = "bad-conflict"
version = "1.0.0"

[[widgets]]
key = "archive"
"#;
    let r = upload(
        &c,
        &format!("{base}/api/admin/themes"),
        &token,
        build_zip(&[("bad-conflict/theme.toml", builtin_conflict)]),
    )
    .await;
    assert_eq!(r.status(), 422);
    assert_eq!(err_code(r).await, "invalid_manifest");

    let bad_param = r#"name = "坏组件"
slug = "bad-param"
version = "1.0.0"

[[widgets]]
key = "notice"

[[widgets.params]]
key = "p"
type = "bogus"
"#;
    let r = upload(
        &c,
        &format!("{base}/api/admin/themes"),
        &token,
        build_zip(&[("bad-param/theme.toml", bad_param)]),
    )
    .await;
    assert_eq!(r.status(), 422);
    assert_eq!(err_code(r).await, "invalid_manifest");
}

// ---------- 4. 主题声明组件 + 按主题隔离 + 删除清理 ----------

#[tokio::test(flavor = "multi_thread")]
async fn theme_widgets_isolation_and_delete_cleanup() {
    let tmp = tempfile::tempdir().unwrap();
    let base = spawn_server(tmp.path().join("config.toml").to_str().unwrap()).await;
    let c = reqwest::Client::new();
    let token = setup_installed(&c, &base, tmp.path()).await;

    // 上传带 [[widgets]] 声明的第三方主题
    let r = upload(
        &c,
        &format!("{base}/api/admin/themes"),
        &token,
        widget_theme_zip(),
    )
    .await;
    assert_eq!(r.status(), 201);

    // 管理端点：notice 以声明默认值出现（source=theme、footer、启用、params 含 text）
    let (status, a) = admin_widgets(&c, &base, &token, "widget-theme").await;
    assert_eq!(status, 200);
    assert_eq!(a["widgets"].as_array().unwrap().len(), 8); // 7 内置 + 1 主题声明
    let notice = find(&a, "notice");
    assert_eq!(notice["source"], "theme");
    assert_eq!(notice["kind"], "custom");
    assert_eq!(notice["label"], "公告栏");
    assert_eq!(notice["enabled"], true);
    assert_eq!(notice["position"], "footer");
    assert_eq!(notice["sort_order"], 70);
    assert_eq!(notice["params"][0]["key"], "text");
    assert_eq!(notice["params"][0]["default"], "欢迎");

    // 公开端点：html 来自主题包文件且完成 {{text}} 替换
    let (_, v) = public_widgets(&c, &base, "widget-theme").await;
    let notice = find(&v, "notice");
    assert_eq!(notice["config"]["html"], "<p class=\"notice\">欢迎</p>");
    // 内置默认启用集也在（与主题无关）
    assert_eq!(
        widget_keys(&v),
        vec!["recent-posts", "tag-cloud", "categories", "notice"]
    );

    // widget-theme 保存自己的配置：改公告文字 + html 后台覆盖 + 停用 tag-cloud
    let (status, saved) = put_widgets(
        &c,
        &base,
        &token,
        "widget-theme",
        json!([
            {"key": "notice", "kind": "custom", "enabled": true, "position": "footer",
             "sort_order": 70, "config": {"text": "你好", "html": "<i>{{text}}</i>"}},
            {"key": "recent-posts", "kind": "builtin", "enabled": true,
             "position": "sidebar", "sort_order": 10},
            {"key": "tag-cloud", "kind": "builtin", "enabled": false,
             "position": "sidebar", "sort_order": 30},
            {"key": "categories", "kind": "builtin", "enabled": true,
             "position": "sidebar", "sort_order": 40},
        ]),
    )
    .await;
    assert_eq!(status, 200, "{saved}");
    let (_, v) = public_widgets(&c, &base, "widget-theme").await;
    assert_eq!(find(&v, "notice")["config"]["html"], "<i>你好</i>");
    assert_eq!(
        widget_keys(&v),
        vec!["recent-posts", "categories", "notice"]
    );

    // default 主题不受影响（隔离）：仍是默认启用集
    let (_, vd) = public_widgets(&c, &base, "default").await;
    assert_eq!(
        widget_keys(&vd),
        vec!["recent-posts", "tag-cloud", "categories"]
    );

    // default 保存自己的配置后切换激活主题，各自不丢
    // （全量替换：显式停用三个默认组件、只启用 archive）
    let (status, _) = put_widgets(
        &c,
        &base,
        &token,
        "default",
        json!([
            {"key": "recent-posts", "kind": "builtin", "enabled": false,
             "position": "sidebar", "sort_order": 10},
            {"key": "tag-cloud", "kind": "builtin", "enabled": false,
             "position": "sidebar", "sort_order": 30},
            {"key": "categories", "kind": "builtin", "enabled": false,
             "position": "sidebar", "sort_order": 40},
            {"key": "archive", "kind": "builtin", "enabled": true,
             "position": "sidebar", "sort_order": 50},
        ]),
    )
    .await;
    assert_eq!(status, 200);
    let r = c
        .post(format!("{base}/api/admin/themes/widget-theme/activate"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    let (_, v) = public_widgets(&c, &base, "widget-theme").await;
    assert_eq!(find(&v, "notice")["config"]["html"], "<i>你好</i>");
    let r = c
        .post(format!("{base}/api/admin/themes/default/activate"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    let (_, vd) = public_widgets(&c, &base, "default").await;
    assert_eq!(widget_keys(&vd), vec!["archive"]); // 全量替换后仅 archive 启用

    // 删除主题 → 组件配置行连带清理；重装后回声明默认值
    let r = c
        .delete(format!("{base}/api/admin/themes/widget-theme"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 204);
    let (status, _) = public_widgets(&c, &base, "widget-theme").await;
    assert_eq!(status, 404); // 磁盘已无该主题
    let r = upload(
        &c,
        &format!("{base}/api/admin/themes"),
        &token,
        widget_theme_zip(),
    )
    .await;
    assert_eq!(r.status(), 201);
    let (_, v) = public_widgets(&c, &base, "widget-theme").await;
    assert_eq!(
        find(&v, "notice")["config"]["html"],
        "<p class=\"notice\">欢迎</p>",
        "重装后应回声明默认值（旧覆盖行已清理）"
    );
    assert_eq!(
        widget_keys(&v),
        vec!["recent-posts", "tag-cloud", "categories", "notice"]
    );
}

// ---------- 5. 未安装门禁白名单 ----------

#[tokio::test(flavor = "multi_thread")]
async fn not_installed_whitelist() {
    let tmp = tempfile::tempdir().unwrap();
    let base = spawn_server(tmp.path().join("config.toml").to_str().unwrap()).await;
    let c = reqwest::Client::new();

    // default：未安装也可读（内置默认启用集），保证安装页/首装前台可渲染
    let (status, v) = public_widgets(&c, &base, "default").await;
    assert_eq!(status, 200);
    assert_eq!(
        widget_keys(&v),
        vec!["recent-posts", "tag-cloud", "categories"]
    );

    // 其他 slug：磁盘不存在 → 404
    let (status, _) = public_widgets(&c, &base, "nope").await;
    assert_eq!(status, 404);

    // 管理端点仍被门禁拦截 → 503 not_installed
    let r = c
        .get(format!("{base}/api/admin/themes/default/widgets"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 503);
    assert_eq!(err_code(r).await, "not_installed");
    let r = c
        .put(format!("{base}/api/admin/themes/default/widgets"))
        .json(&json!({ "widgets": [] }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 503);
    // 公开 GET 非白名单方法（PUT /api/themes/:slug/widgets）→ 503
    let r = c
        .put(format!("{base}/api/themes/default/widgets"))
        .json(&json!({}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 503);
}

// ---------- 6. 配套公开接口：site/stats 与 posts?order=hot ----------

#[tokio::test(flavor = "multi_thread")]
async fn site_stats_and_hot_order() {
    let tmp = tempfile::tempdir().unwrap();
    let base = spawn_server(tmp.path().join("config.toml").to_str().unwrap()).await;
    let c = reqwest::Client::new();
    let token = setup_installed(&c, &base, tmp.path()).await;

    // 建两篇文章：A 带 2 条评论、B 无评论（安装时注入了示例内容，计数用相对比较）
    for (title, slug_out) in [("HotTestA", "a"), ("HotTestB", "b")] {
        let r = c
            .post(format!("{base}/api/admin/posts"))
            .bearer_auth(&token)
            .json(&json!({
                "title": title, "content_md": "正文", "status": "published"
            }))
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), 201);
        let _ = slug_out;
    }
    let posts: Value = c
        .get(format!(
            "{base}/api/admin/posts?status=published&per_page=100"
        ))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let slug_a = posts["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["title"] == "HotTestA")
        .unwrap()["slug"]
        .as_str()
        .unwrap()
        .to_string();
    for i in 0..2 {
        let r = c
            .post(format!("{base}/api/posts/{slug_a}/comments"))
            .json(&json!({"author_name": "路人", "content": format!("评论 {i}")}))
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), 201);
    }

    // order=hot：A（2 评论）排在 B（0 评论）之前
    let hot: Value = c
        .get(format!("{base}/api/posts?order=hot&per_page=100"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let items = hot["items"].as_array().unwrap();
    let pos = |t: &str| items.iter().position(|p| p["title"] == t).unwrap();
    assert!(pos("HotTestA") < pos("HotTestB"));
    let a = &items[pos("HotTestA")];
    assert!(a["comment_count"].as_i64().unwrap() >= 2);

    // order=recent（默认）：按 published_at DESC，B 后发布应在 A 前（同秒时不稳定，
    // 只断言接口 200 且形状一致）
    let recent: Value = c
        .get(format!("{base}/api/posts?order=recent&per_page=5"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(recent["per_page"], 5);

    // 非法 order → 422 validation_error
    let r = c
        .get(format!("{base}/api/posts?order=bogus"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 422);
    assert_eq!(err_code(r).await, "validation_error");

    // site/stats：published 文章数与 /api/posts total 一致；评论数 ≥2；installed_at 为 RFC3339
    let stats: Value = c
        .get(format!("{base}/api/site/stats"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let listed: Value = c
        .get(format!("{base}/api/posts?per_page=1"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(stats["post_count"], listed["total"]);
    assert!(stats["comment_count"].as_i64().unwrap() >= 2);
    let installed_at = stats["installed_at"].as_str().unwrap();
    assert!(
        installed_at.ends_with('Z') && installed_at.len() >= 20,
        "installed_at 应为 RFC3339 UTC: {installed_at}"
    );

    // stats 不进白名单：未安装的新服务上 → 503
    let tmp2 = tempfile::tempdir().unwrap();
    let base2 = spawn_server(tmp2.path().join("config.toml").to_str().unwrap()).await;
    let r = c
        .get(format!("{base2}/api/site/stats"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 503);
    assert_eq!(err_code(r).await, "not_installed");
}
