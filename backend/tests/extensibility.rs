//! 扩展系统集成测试（docs/extensibility-contract.md）：
//! - 插件：安装 zip → 启用 → 钩子生效（before_render/after_render/before_create/after_publish）
//!   → 前端注入 → 停用 → 删除；script_error / last_error / 重启恢复
//! - 主题：安装 zip → 激活 → themes/active 返回新令牌；default 内置保护；静态托管防穿越
//! - 未安装门禁白名单：themes/active、theme.css、assets、frontend/injections
//!
//! 示例插件/主题 zip 在测试内构造（跨平台、无二进制 fixture 依赖）；
//! 运行 `cargo test --test extensibility generate_fixtures -- --ignored`
//! 可把同样的 zip 落盘到 tests/fixtures/ 备用。

use reedblog_backend::state::AppState;
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

fn install_body(dir: &Path) -> Value {
    json!({
        "db_type": "sqlite",
        "sqlite_path": dir.join("reedblog.db").to_str().unwrap(),
        "admin": {"username": "admin", "password": "secret123"},
        "site": {"title": "扩展测试博客"}
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

// ---------- 示例插件 fixture ----------

const HELLO_MANIFEST: &str = r#"name = "测试插件"
slug = "hello-plugin"
version = "1.0.0"
description = "集成测试用最小示例插件"
author = "reedblog-test"
hooks = ["post.before_render", "post.after_render", "comment.before_create", "post.after_publish"]
inject = ["head", "body_end"]
"#;

const HELLO_SCRIPT: &str = r#"
fn post_before_render(ctx) {
    ctx.content_md = ctx.content_md + "\n\nBEFORE_RENDER_MARK";
    ctx
}

fn post_after_render(ctx) {
    ctx.content_html = ctx.content_html + "<!-- AFTER_RENDER_MARK -->";
    ctx
}

fn comment_before_create(ctx) {
    if ctx.content.contains("spam") {
        #{ action: "block", reason: "垃圾评论被禁止" }
    } else {
        #{ action: "allow", author_name: ctx.author_name + "[已过审]", content: ctx.content }
    }
}

fn post_after_publish(ctx) {
    ctx.title
}
"#;

const HELLO_HEAD: &str = r#"<meta name="hello-plugin" content="HEAD_INJECT_OK">"#;
const HELLO_BODY_END: &str = r#"<div id="hello-widget">BODY_END_INJECT_OK</div>"#;

fn hello_plugin_zip() -> Vec<u8> {
    build_zip(&[
        ("hello-plugin/manifest.toml", HELLO_MANIFEST),
        ("hello-plugin/main.rhai", HELLO_SCRIPT),
        ("hello-plugin/inject/head.html", HELLO_HEAD),
        ("hello-plugin/inject/body_end.html", HELLO_BODY_END),
        ("hello-plugin/README.md", "# 示例插件"),
    ])
}

// ---------- 示例主题 fixture ----------

const MY_THEME_TOML: &str = r#"name = "测试主题"
slug = "my-theme"
version = "2.1.0"
description = "集成测试用最小示例主题"
author = "reedblog-test"

[tokens]
background = "10 20% 30%"
foreground = "40 50% 60%"
primary_foreground = "0 0% 98%"
radius = "0.75rem"

[tokens_dark]
background = "0 0% 5%"
"#;

fn my_theme_zip() -> Vec<u8> {
    build_zip(&[
        ("my-theme/theme.toml", MY_THEME_TOML),
        ("my-theme/theme.css", "body { background: #123456; }"),
        ("my-theme/assets/logo.txt", "THEME_ASSET_OK"),
    ])
}

/// 把示例 zip 落盘到 tests/fixtures/（默认忽略；需要时手动运行）
#[test]
#[ignore]
fn generate_fixtures() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("hello-plugin.zip"), hello_plugin_zip()).unwrap();
    std::fs::write(dir.join("my-theme.zip"), my_theme_zip()).unwrap();
    println!("fixtures written to {}", dir.display());
}

// ---------- 1. 插件主验收链路：安装→启用→钩子生效→注入→停用→删除 ----------

#[tokio::test(flavor = "multi_thread")]
async fn plugin_lifecycle_hooks_and_injections() {
    let tmp = tempfile::tempdir().unwrap();
    let cfg_path = tmp.path().join("config.toml");
    let base = spawn_server(cfg_path.to_str().unwrap()).await;
    let c = reqwest::Client::new();

    // 未安装：injections 可用且为空（白名单）
    let r = c
        .get(format!("{base}/api/frontend/injections"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    assert_eq!(
        r.json::<Value>().await.unwrap(),
        json!({"head": [], "body_end": []})
    );

    let token = setup_installed(&c, &base, tmp.path()).await;

    // 管理端点无 token → 401
    let r = c
        .get(format!("{base}/api/admin/plugins"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 401);

    // 初始列表为空
    let v = c
        .get(format!("{base}/api/admin/plugins"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    assert_eq!(v["total"], 0);
    assert_eq!(v["items"].as_array().unwrap().len(), 0);

    // 上传安装 → 201，默认 enabled:false（防误触发）
    let r = upload(
        &c,
        &format!("{base}/api/admin/plugins"),
        &token,
        hello_plugin_zip(),
    )
    .await;
    assert_eq!(r.status(), 201, "{}", r.text().await.unwrap());
    let r2 = c
        .get(format!("{base}/api/admin/plugins/hello-plugin"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(r2.status(), 200);
    let info: Value = r2.json().await.unwrap();
    assert_eq!(info["slug"], "hello-plugin");
    assert_eq!(info["name"], "测试插件");
    assert_eq!(info["version"], "1.0.0");
    assert_eq!(info["enabled"], false);
    assert_eq!(info["hooks"].as_array().unwrap().len(), 4);
    assert_eq!(info["inject"], json!(["head", "body_end"]));
    assert!(info["installed_at"].as_str().unwrap().ends_with('Z'));
    assert!(info.get("last_error").is_none() || info["last_error"].is_null());

    // 重复安装同 slug → 409 plugin_exists
    let r = upload(
        &c,
        &format!("{base}/api/admin/plugins"),
        &token,
        hello_plugin_zip(),
    )
    .await;
    assert_eq!(r.status(), 409);
    assert_eq!(err_code(r).await, "plugin_exists");

    // 非 zip → 422 invalid_package
    let r = upload(
        &c,
        &format!("{base}/api/admin/plugins"),
        &token,
        b"definitely not a zip".to_vec(),
    )
    .await;
    assert_eq!(r.status(), 422);
    assert_eq!(err_code(r).await, "invalid_package");

    // 缺 main.rhai → 422 invalid_package
    let r = upload(
        &c,
        &format!("{base}/api/admin/plugins"),
        &token,
        build_zip(&[("no-script/manifest.toml", HELLO_MANIFEST.replace("hello-plugin", "no-script").as_str())]),
    )
    .await;
    assert_eq!(r.status(), 422);
    assert_eq!(err_code(r).await, "invalid_package");

    // 未知钩子 → 422 invalid_manifest
    let bad_manifest = "name = \"x\"\nslug = \"bad-hooks\"\nversion = \"1.0.0\"\nhooks = [\"post.bogus\"]\n";
    let r = upload(
        &c,
        &format!("{base}/api/admin/plugins"),
        &token,
        build_zip(&[("bad-hooks/manifest.toml", bad_manifest), ("bad-hooks/main.rhai", "// empty")]),
    )
    .await;
    assert_eq!(r.status(), 422);
    assert_eq!(err_code(r).await, "invalid_manifest");

    // slug 与目录名不一致 → 422 invalid_manifest
    let r = upload(
        &c,
        &format!("{base}/api/admin/plugins"),
        &token,
        build_zip(&[
            ("mismatch-dir/manifest.toml", HELLO_MANIFEST),
            ("mismatch-dir/main.rhai", "// empty"),
        ]),
    )
    .await;
    assert_eq!(r.status(), 422);
    assert_eq!(err_code(r).await, "invalid_manifest");

    // 启用 → 200 enabled:true
    let r = c
        .post(format!("{base}/api/admin/plugins/hello-plugin/enable"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    assert_eq!(r.json::<Value>().await.unwrap()["enabled"], true);

    // 启用不存在的插件 → 404
    let r = c
        .post(format!("{base}/api/admin/plugins/nope/enable"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 404);

    // 前端注入：head/body_end 片段按声明返回
    let v = c
        .get(format!("{base}/api/frontend/injections"))
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    assert_eq!(v["head"].as_array().unwrap().len(), 1);
    assert_eq!(v["head"][0]["plugin"], "hello-plugin");
    assert_eq!(v["head"][0]["html"].as_str().unwrap(), HELLO_HEAD);
    assert_eq!(v["body_end"][0]["html"].as_str().unwrap(), HELLO_BODY_END);

    // 发一篇已发布文章（触发 post.after_publish，通知类、无可见输出）
    let r = c
        .post(format!("{base}/api/admin/posts"))
        .bearer_auth(&token)
        .json(&json!({
            "title": "Hook Test", "slug": "hook-test",
            "content_md": "Hello **world**", "status": "published"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 201);

    // 钩子生效：before_render 改写 content_md → 渲染 → after_render 追加内容进 HTML
    let v = c
        .get(format!("{base}/api/posts/hook-test"))
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    assert!(
        v["content_md"].as_str().unwrap().contains("BEFORE_RENDER_MARK"),
        "before_render 应改写 content_md: {v}"
    );
    let html = v["content_html"].as_str().unwrap();
    assert!(html.contains("<strong>world</strong>"), "markdown 应被渲染: {html}");
    assert!(html.contains("AFTER_RENDER_MARK"), "after_render 追加文本应出现在文章 HTML: {html}");

    // after_publish 正常执行后 last_error 保持为空
    let v = c
        .get(format!("{base}/api/admin/plugins/hello-plugin"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    assert!(v.get("last_error").is_none() || v["last_error"].is_null());

    // comment.before_create：block 短路 → 403 comment_blocked，reason 进 message
    let r = c
        .post(format!("{base}/api/posts/hook-test/comments"))
        .json(&json!({"author_name": "spammer", "content": "buy spam now"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 403);
    let body: Value = r.json().await.unwrap();
    assert_eq!(body["error"]["code"], "comment_blocked");
    assert_eq!(body["error"]["message"], "垃圾评论被禁止");

    // comment.before_create：allow 可携带修改后字段
    let r = c
        .post(format!("{base}/api/posts/hook-test/comments"))
        .json(&json!({"author_name": "读者甲", "content": "好文"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 201);
    let cm: Value = r.json().await.unwrap();
    assert_eq!(cm["author_name"], "读者甲[已过审]");
    assert_eq!(cm["content"], "好文");

    // 停用 → 200 enabled:false；钩子与注入立即失效（热切换，无需重启）
    let r = c
        .post(format!("{base}/api/admin/plugins/hello-plugin/disable"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    assert_eq!(r.json::<Value>().await.unwrap()["enabled"], false);

    let v = c
        .get(format!("{base}/api/frontend/injections"))
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    assert_eq!(v, json!({"head": [], "body_end": []}));

    let v = c
        .get(format!("{base}/api/posts/hook-test"))
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    assert!(!v["content_html"].as_str().unwrap().contains("AFTER_RENDER_MARK"));
    assert!(!v["content_md"].as_str().unwrap().contains("BEFORE_RENDER_MARK"));

    // 停用后评论恢复原样
    let r = c
        .post(format!("{base}/api/posts/hook-test/comments"))
        .json(&json!({"author_name": "读者乙", "content": "buy spam now"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 201);
    assert_eq!(r.json::<Value>().await.unwrap()["author_name"], "读者乙");

    // 删除（停用中）→ 204；列表清空；再删/再查 → 404
    let r = c
        .delete(format!("{base}/api/admin/plugins/hello-plugin"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 204);
    let v = c
        .get(format!("{base}/api/admin/plugins"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    assert_eq!(v["total"], 0);
    let r = c
        .get(format!("{base}/api/admin/plugins/hello-plugin"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 404);
    let r = c
        .delete(format!("{base}/api/admin/plugins/hello-plugin"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 404);
    // 磁盘目录已删除
    assert!(!tmp.path().join("plugins/hello-plugin").exists());

    // 删除后可重新安装（slug 释放）
    let r = upload(
        &c,
        &format!("{base}/api/admin/plugins"),
        &token,
        hello_plugin_zip(),
    )
    .await;
    assert_eq!(r.status(), 201);
}

// ---------- 2. 主题：安装→激活→令牌下发；default 内置保护；静态托管 ----------

#[tokio::test(flavor = "multi_thread")]
async fn theme_lifecycle_and_static_serving() {
    let tmp = tempfile::tempdir().unwrap();
    let cfg_path = tmp.path().join("config.toml");
    let base = spawn_server(cfg_path.to_str().unwrap()).await;
    let c = reqwest::Client::new();
    let token = setup_installed(&c, &base, tmp.path()).await;

    // 安装时已自动生成内置 default 主题（shadcn neutral 令牌，原样下发）
    let v = c
        .get(format!("{base}/api/themes/active"))
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    assert_eq!(v["slug"], "default");
    assert_eq!(v["tokens"]["background"], "0 0% 100%");
    assert_eq!(v["tokens"]["primary"], "0 0% 9%");
    assert_eq!(v["tokens"]["radius"], "0.5rem");
    assert!(v["css_url"].is_null(), "内置 default 无 theme.css");
    assert!(tmp.path().join("themes/default/theme.toml").is_file());

    // 管理列表：default builtin=true active=true
    let v = c
        .get(format!("{base}/api/admin/themes"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    assert_eq!(v["total"], 1);
    assert_eq!(v["items"][0]["slug"], "default");
    assert_eq!(v["items"][0]["builtin"], true);
    assert_eq!(v["items"][0]["active"], true);
    assert_eq!(v["items"][0]["has_css"], false);

    // default 不可删除 → 409 builtin_protected
    let r = c
        .delete(format!("{base}/api/admin/themes/default"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 409);
    assert_eq!(err_code(r).await, "builtin_protected");

    // 上传覆盖 default → 409 builtin_protected
    let default_overwrite = build_zip(&[(
        "default/theme.toml",
        "name = \"假默认\"\nslug = \"default\"\nversion = \"9.9.9\"\n",
    )]);
    let r = upload(&c, &format!("{base}/api/admin/themes"), &token, default_overwrite).await;
    assert_eq!(r.status(), 409);
    assert_eq!(err_code(r).await, "builtin_protected");

    // 安装 my-theme → 201
    let r = upload(&c, &format!("{base}/api/admin/themes"), &token, my_theme_zip()).await;
    let status = r.status();
    let info: Value = r.json().await.unwrap();
    assert_eq!(status, 201, "{info}");
    assert_eq!(info["slug"], "my-theme");
    assert_eq!(info["version"], "2.1.0");
    assert_eq!(info["active"], false);
    assert_eq!(info["builtin"], false);
    assert_eq!(info["has_css"], true);

    // 重复安装 → 409 theme_exists
    let r = upload(&c, &format!("{base}/api/admin/themes"), &token, my_theme_zip()).await;
    assert_eq!(r.status(), 409);
    assert_eq!(err_code(r).await, "theme_exists");

    // 非 zip → 422 invalid_package；缺 theme.toml → invalid_package；坏 toml → invalid_manifest
    let r = upload(
        &c,
        &format!("{base}/api/admin/themes"),
        &token,
        b"not a zip".to_vec(),
    )
    .await;
    assert_eq!(r.status(), 422);
    assert_eq!(err_code(r).await, "invalid_package");
    let r = upload(
        &c,
        &format!("{base}/api/admin/themes"),
        &token,
        build_zip(&[("x-theme/readme.txt", "hi")]),
    )
    .await;
    assert_eq!(r.status(), 422);
    assert_eq!(err_code(r).await, "invalid_package");
    let r = upload(
        &c,
        &format!("{base}/api/admin/themes"),
        &token,
        build_zip(&[("bad-toml/theme.toml", "name = = [ broken")]),
    )
    .await;
    assert_eq!(r.status(), 422);
    assert_eq!(err_code(r).await, "invalid_manifest");

    // theme.css 静态托管：text/css
    let r = c
        .get(format!("{base}/api/themes/my-theme/theme.css"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    assert!(r
        .headers()
        .get("content-type")
        .unwrap()
        .to_str()
        .unwrap()
        .starts_with("text/css"));
    assert_eq!(r.text().await.unwrap(), "body { background: #123456; }");
    // 不存在的主题/css → 404
    let r = c
        .get(format!("{base}/api/themes/no-such/theme.css"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 404);
    let r = c
        .get(format!("{base}/api/themes/default/theme.css"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 404);

    // assets 静态托管 + 目录穿越防护
    let r = c
        .get(format!("{base}/api/themes/my-theme/assets/logo.txt"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    assert_eq!(r.text().await.unwrap(), "THEME_ASSET_OK");
    for attack in [
        "/api/themes/my-theme/assets/..%2Ftheme.toml",
        "/api/themes/my-theme/assets/..%2F..%2Fconfig.toml",
        "/api/themes/my-theme/assets/%2e%2e/theme.toml",
    ] {
        let r = c.get(format!("{base}{attack}")).send().await.unwrap();
        assert_ne!(r.status(), 200, "目录穿越应被拒绝: {attack}");
        let text = r.text().await.unwrap();
        assert!(!text.contains("测试主题"), "不得泄漏 theme.toml: {attack}");
    }

    // 激活 my-theme → 200；themes/active 返回新令牌（原样下发，含 tokens_dark）
    let r = c
        .post(format!("{base}/api/admin/themes/my-theme/activate"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    assert_eq!(r.json::<Value>().await.unwrap()["active"], true);

    let v = c
        .get(format!("{base}/api/themes/active"))
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    assert_eq!(v["slug"], "my-theme");
    assert_eq!(v["name"], "测试主题");
    assert_eq!(v["tokens"]["background"], "10 20% 30%");
    assert_eq!(v["tokens"]["radius"], "0.75rem");
    assert_eq!(v["tokens_dark"]["background"], "0 0% 5%");
    assert_eq!(v["css_url"], "/api/themes/my-theme/theme.css");

    // active 权威来源 = config.toml
    let cfg_text = std::fs::read_to_string(&cfg_path).unwrap();
    assert!(cfg_text.contains("active = \"my-theme\""), "{cfg_text}");

    // 激活中的主题不可删 → 409 theme_active
    let r = c
        .delete(format!("{base}/api/admin/themes/my-theme"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 409);
    assert_eq!(err_code(r).await, "theme_active");

    // 激活不存在的主题 → 404
    let r = c
        .post(format!("{base}/api/admin/themes/nope/activate"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 404);

    // 切回 default → my-theme 可删 → 204
    let r = c
        .post(format!("{base}/api/admin/themes/default/activate"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    let r = c
        .delete(format!("{base}/api/admin/themes/my-theme"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 204);
    assert!(!tmp.path().join("themes/my-theme").exists());
    let v = c
        .get(format!("{base}/api/admin/themes"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    assert_eq!(v["total"], 1);
}

// ---------- 3. script_error / last_error / 重启恢复 enabled ----------

#[tokio::test(flavor = "multi_thread")]
async fn plugin_script_errors_and_restart_restore() {
    let tmp = tempfile::tempdir().unwrap();
    let cfg_path = tmp.path().join("config.toml");
    let cfg_str = cfg_path.to_str().unwrap();
    let base1 = spawn_server(cfg_str).await;
    let c = reqwest::Client::new();
    let token = setup_installed(&c, &base1, tmp.path()).await;

    // 语法错误插件：enable → 422 script_error，保持未启用
    let r = upload(
        &c,
        &format!("{base1}/api/admin/plugins"),
        &token,
        build_zip(&[
            (
                "bad-syntax/manifest.toml",
                "name = \"语法坏\"\nslug = \"bad-syntax\"\nversion = \"1.0.0\"\nhooks = [\"post.after_render\"]\n",
            ),
            ("bad-syntax/main.rhai", "fn post_after_render(ctx) { ctx.content_html = ; }"),
        ]),
    )
    .await;
    assert_eq!(r.status(), 201);
    let r = c
        .post(format!("{base1}/api/admin/plugins/bad-syntax/enable"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 422);
    assert_eq!(err_code(r).await, "script_error");
    let v = c
        .get(format!("{base1}/api/admin/plugins/bad-syntax"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    assert_eq!(v["enabled"], false);

    // min_app_version 高于当前版本：拒绝启用
    let r = upload(
        &c,
        &format!("{base1}/api/admin/plugins"),
        &token,
        build_zip(&[
            (
                "future/manifest.toml",
                "name = \"未来插件\"\nslug = \"future\"\nversion = \"1.0.0\"\nmin_app_version = \"99.0.0\"\nhooks = []\n",
            ),
            ("future/main.rhai", "// nothing"),
        ]),
    )
    .await;
    assert_eq!(r.status(), 201);
    let r = c
        .post(format!("{base1}/api/admin/plugins/future/enable"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 422);
    assert_eq!(err_code(r).await, "invalid_manifest");

    // 运行时错误插件：语法合法可启用，但钩子执行抛错
    let r = upload(
        &c,
        &format!("{base1}/api/admin/plugins"),
        &token,
        build_zip(&[
            (
                "bad-runtime/manifest.toml",
                "name = \"运行时坏\"\nslug = \"bad-runtime\"\nversion = \"1.0.0\"\nhooks = [\"post.after_render\"]\n",
            ),
            ("bad-runtime/main.rhai", "fn post_after_render(ctx) { throw(\"BOOM_RUNTIME\"); }"),
        ]),
    )
    .await;
    assert_eq!(r.status(), 201);
    let r = c
        .post(format!("{base1}/api/admin/plugins/bad-runtime/enable"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);

    // hello-plugin 也装上并启用（链式：bad-runtime 字典序在前、先炸；hello-plugin 继续生效）
    let r = upload(
        &c,
        &format!("{base1}/api/admin/plugins"),
        &token,
        hello_plugin_zip(),
    )
    .await;
    assert_eq!(r.status(), 201);
    let r = c
        .post(format!("{base1}/api/admin/plugins/hello-plugin/enable"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);

    let r = c
        .post(format!("{base1}/api/admin/posts"))
        .bearer_auth(&token)
        .json(&json!({
            "title": "Chain Test", "slug": "chain-test",
            "content_md": "body", "status": "published"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 201);

    // 运行时错误不阻断主流程：文章照常返回，hello-plugin 的追加仍在
    let v = c
        .get(format!("{base1}/api/posts/chain-test"))
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    let html = v["content_html"].as_str().unwrap();
    assert!(html.contains("AFTER_RENDER_MARK"), "{html}");

    // bad-runtime 详情暴露 last_error；hello-plugin 无
    let v = c
        .get(format!("{base1}/api/admin/plugins/bad-runtime"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    let last_error = v["last_error"].as_str().unwrap_or_default();
    assert!(last_error.contains("BOOM_RUNTIME"), "{last_error}");
    assert!(last_error.contains("post.after_render"), "{last_error}");
    let v = c
        .get(format!("{base1}/api/admin/plugins/hello-plugin"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    assert!(v.get("last_error").is_none() || v["last_error"].is_null());

    // ---- 模拟重启：startup_state 按 DB 恢复 enabled ----
    let state2 = reedblog_backend::startup_state(cfg_str).await;
    assert!(state2.is_installed().await);
    let app2 = reedblog_backend::build_router(state2, vec!["http://localhost:5173".to_string()]);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base2 = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
    tokio::spawn(async move {
        axum::serve(listener, app2).await.unwrap();
    });

    let v = c
        .get(format!("{base2}/api/admin/plugins"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    assert_eq!(v["total"], 4);
    let by_slug = |s: &str| -> Value {
        v["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|i| i["slug"] == s)
            .unwrap_or_else(|| panic!("缺少插件 {s}"))
            .clone()
    };
    assert_eq!(by_slug("hello-plugin")["enabled"], true);
    assert_eq!(by_slug("bad-runtime")["enabled"], true);
    assert_eq!(by_slug("bad-syntax")["enabled"], false);
    assert_eq!(by_slug("future")["enabled"], false);

    // 重启后钩子依然生效；注入依然返回
    let v = c
        .get(format!("{base2}/api/posts/chain-test"))
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    assert!(v["content_html"].as_str().unwrap().contains("AFTER_RENDER_MARK"));
    let v = c
        .get(format!("{base2}/api/frontend/injections"))
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    assert_eq!(v["head"][0]["plugin"], "hello-plugin");

    // 启用中的插件也可直接删除 → 204，且钩子立即失效
    let r = c
        .delete(format!("{base2}/api/admin/plugins/hello-plugin"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 204);
    let v = c
        .get(format!("{base2}/api/posts/chain-test"))
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    assert!(!v["content_html"].as_str().unwrap().contains("AFTER_RENDER_MARK"));
}

// ---------- 4. 未安装门禁白名单（扩展契约第三部分） ----------

#[tokio::test(flavor = "multi_thread")]
async fn extensibility_not_installed_whitelist() {
    let tmp = tempfile::tempdir().unwrap();
    let base = spawn_server(tmp.path().join("config.toml").to_str().unwrap()).await;
    let c = reqwest::Client::new();

    // themes/active：未安装（磁盘无主题目录）也返回内置 default 令牌兜底
    let r = c
        .get(format!("{base}/api/themes/active"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    let v: Value = r.json().await.unwrap();
    assert_eq!(v["slug"], "default");
    assert_eq!(v["tokens"]["background"], "0 0% 100%");

    // frontend/injections：未安装返回空
    let r = c
        .get(format!("{base}/api/frontend/injections"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    assert_eq!(
        r.json::<Value>().await.unwrap(),
        json!({"head": [], "body_end": []})
    );

    // theme.css / assets：未安装不再 503（文件不存在则 404）
    let r = c
        .get(format!("{base}/api/themes/default/theme.css"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 404);
    let r = c
        .get(format!("{base}/api/themes/x/assets/y.txt"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 404);

    // 管理端点仍被门禁拦截 → 503 not_installed
    let r = c
        .get(format!("{base}/api/admin/plugins"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 503);
    assert_eq!(err_code(r).await, "not_installed");
    let r = c
        .get(format!("{base}/api/admin/themes"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 503);
    // 非 GET 的白名单路径不放行
    let r = c
        .delete(format!("{base}/api/themes/active"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 503);
}
