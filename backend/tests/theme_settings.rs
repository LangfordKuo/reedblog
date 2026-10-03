//! 主题设置集成测试（docs/extensibility-contract.md「主题设置项 / 主题设置 API」）：
//! - 生效值读取：声明 default 合并、公开端点与管理 panel 形状
//! - PUT 保存/回读、类型规范化（color 小写、switch/number 类型转换）
//! - 校验：未声明 key → 422 unknown_setting；select 越界 / color 非 hex /
//!   switch 非布尔 → 422 invalid_value；声明非法的 zip 上传 → 422 invalid_manifest
//! - 按 slug 隔离：切换主题各自设置不丢；删除主题连带清理设置（重装回默认值）
//! - 未安装门禁白名单：GET /api/themes/default/settings 可用（values=默认值），
//!   其余 slug 404，PUT/panel 503
//!
//! 第三方主题包直接读 tests/fixtures/my-theme.zip（带 [[settings]] 声明；
//! 由 extensibility.rs 的 generate_fixtures 生成，与该文件内 MY_THEME_TOML 同源）。

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
        "site": {"title": "主题设置测试博客"}
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

/// tests/fixtures/ 下的示例包
fn fixture_zip(name: &str) -> Vec<u8> {
    std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(name),
    )
    .unwrap_or_else(|e| panic!("读取 fixture {name} 失败: {e}"))
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

/// GET 公开生效值
async fn public_settings(c: &reqwest::Client, base: &str, slug: &str) -> (u16, Value) {
    let r = c
        .get(format!("{base}/api/themes/{slug}/settings"))
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

/// PUT 保存设置，返回 (status, body)
async fn put_settings(
    c: &reqwest::Client,
    base: &str,
    token: &str,
    slug: &str,
    values: Value,
) -> (u16, Value) {
    let r = c
        .put(format!("{base}/api/admin/themes/{slug}/settings"))
        .bearer_auth(token)
        .json(&json!({ "values": values }))
        .send()
        .await
        .unwrap();
    let status = r.status().as_u16();
    (status, r.json().await.unwrap())
}

// ---------- 1. 默认值读取 + 保存/回读 + panel 形状 ----------

#[tokio::test(flavor = "multi_thread")]
async fn defaults_save_and_readback() {
    let tmp = tempfile::tempdir().unwrap();
    let base = spawn_server(tmp.path().join("config.toml").to_str().unwrap()).await;
    let c = reqwest::Client::new();
    let token = setup_installed(&c, &base, tmp.path()).await;

    // 内置 default 主题声明了 layout / wide_layout / accent_color 三项设置
    let (status, v) = public_settings(&c, &base, "default").await;
    assert_eq!(status, 200);
    assert_eq!(v["slug"], "default");
    let decls = v["settings"].as_array().unwrap();
    assert_eq!(decls.len(), 3);
    let layout = decls.iter().find(|d| d["key"] == "layout").unwrap();
    assert_eq!(layout["type"], "select");
    assert_eq!(layout["group"], "布局");
    let opts = layout["options"].as_array().unwrap();
    assert_eq!(opts[0]["value"], "topbar-two-column");
    assert_eq!(opts[1]["value"], "topbar-minimal-three-column");
    assert_ne!(
        opts[1]["label"], opts[1]["value"],
        "options 应归一化为 {{value,label}}"
    );
    // 未保存过时 values = 声明默认值（类型化输出：switch → bool）
    assert_eq!(v["values"]["layout"], "topbar-two-column");
    assert_eq!(v["values"]["wide_layout"], false);
    assert_eq!(v["values"]["accent_color"], "#0f172a");

    // 管理 panel：当前激活主题（default）的声明 + 已存值
    let r = c
        .get(format!("{base}/api/admin/themes/active/settings-panel"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    let p: Value = r.json().await.unwrap();
    assert_eq!(p["slug"], "default");
    assert_eq!(p["name"], "默认主题");
    assert_eq!(p["settings"], v["settings"]);
    assert_eq!(p["values"], v["values"]);
    // 无 token → 401
    let r = c
        .get(format!("{base}/api/admin/themes/active/settings-panel"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 401);

    // 保存：切三列布局 + 改配色 + 开宽幅（color 大写 hex → 规范化小写回显）
    let (status, saved) = put_settings(
        &c,
        &base,
        &token,
        "default",
        json!({
            "layout": "topbar-minimal-three-column",
            "accent_color": "#FF0000",
            "wide_layout": true,
        }),
    )
    .await;
    assert_eq!(status, 200, "{saved}");
    assert_eq!(saved["values"]["layout"], "topbar-minimal-three-column");
    assert_eq!(saved["values"]["accent_color"], "#ff0000");
    assert_eq!(saved["values"]["wide_layout"], true);

    // 公开端点回读一致；panel 同步
    let (_, v2) = public_settings(&c, &base, "default").await;
    assert_eq!(v2["values"], saved["values"]);
    let p: Value = c
        .get(format!("{base}/api/admin/themes/active/settings-panel"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(p["values"], saved["values"]);

    // 部分更新语义：只提交一个 key，其余已存值保持
    let (status, saved2) = put_settings(
        &c,
        &base,
        &token,
        "default",
        json!({ "wide_layout": false }),
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(saved2["values"]["wide_layout"], false);
    assert_eq!(saved2["values"]["layout"], "topbar-minimal-three-column");
    assert_eq!(saved2["values"]["accent_color"], "#ff0000");
}

// ---------- 2. 校验：unknown_setting / invalid_value / 404 / 401 ----------

#[tokio::test(flavor = "multi_thread")]
async fn validation_errors() {
    let tmp = tempfile::tempdir().unwrap();
    let base = spawn_server(tmp.path().join("config.toml").to_str().unwrap()).await;
    let c = reqwest::Client::new();
    let token = setup_installed(&c, &base, tmp.path()).await;

    // 未声明 key → 422 unknown_setting
    let (status, body) =
        put_settings(&c, &base, &token, "default", json!({"no_such_key": "x"})).await;
    assert_eq!(status, 422);
    assert_eq!(body["error"]["code"], "unknown_setting");

    // select 越界 → 422 invalid_value
    let (status, body) = put_settings(
        &c,
        &base,
        &token,
        "default",
        json!({"layout": "sidebar-only"}),
    )
    .await;
    assert_eq!(status, 422);
    assert_eq!(body["error"]["code"], "invalid_value");

    // color 非 hex → 422 invalid_value
    let (status, body) =
        put_settings(&c, &base, &token, "default", json!({"accent_color": "red"})).await;
    assert_eq!(status, 422);
    assert_eq!(body["error"]["code"], "invalid_value");

    // switch 非布尔 → 422 invalid_value
    let (status, body) =
        put_settings(&c, &base, &token, "default", json!({"wide_layout": "yes"})).await;
    assert_eq!(status, 422);
    assert_eq!(body["error"]["code"], "invalid_value");

    // text 类超长 → 422 invalid_value（用 fixture 主题的 textarea 设置验证）
    let r = upload(
        &c,
        &format!("{base}/api/admin/themes"),
        &token,
        fixture_zip("my-theme.zip"),
    )
    .await;
    assert_eq!(r.status(), 201);
    let (status, body) = put_settings(
        &c,
        &base,
        &token,
        "my-theme",
        json!({"notice": "长".repeat(5001)}),
    )
    .await;
    assert_eq!(status, 422);
    assert_eq!(body["error"]["code"], "invalid_value");

    // values 非对象 → 422 validation_error
    let r = c
        .put(format!("{base}/api/admin/themes/default/settings"))
        .bearer_auth(&token)
        .json(&json!({ "values": "layout" }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 422);
    assert_eq!(err_code(r).await, "validation_error");

    // 不存在的主题 → 404；无 token → 401
    let (status, _) = put_settings(&c, &base, &token, "nope", json!({})).await;
    assert_eq!(status, 404);
    let r = c
        .put(format!("{base}/api/admin/themes/default/settings"))
        .json(&json!({ "values": {} }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 401);

    // settings 声明非法的 zip → 上传 422 invalid_manifest
    let bad_type = r#"name = "坏设置"
slug = "bad-settings"
version = "1.0.0"

[[settings]]
key = "a"
type = "bogus"
"#;
    let r = upload(
        &c,
        &format!("{base}/api/admin/themes"),
        &token,
        build_zip(&[("bad-settings/theme.toml", bad_type)]),
    )
    .await;
    assert_eq!(r.status(), 422);
    assert_eq!(err_code(r).await, "invalid_manifest");

    let select_no_options = r#"name = "坏设置"
slug = "bad-select"
version = "1.0.0"

[[settings]]
key = "a"
type = "select"
default = "x"
"#;
    let r = upload(
        &c,
        &format!("{base}/api/admin/themes"),
        &token,
        build_zip(&[("bad-select/theme.toml", select_no_options)]),
    )
    .await;
    assert_eq!(r.status(), 422);
    assert_eq!(err_code(r).await, "invalid_manifest");

    let dup_key = r#"name = "坏设置"
slug = "bad-dup"
version = "1.0.0"

[[settings]]
key = "a"
type = "text"

[[settings]]
key = "a"
type = "text"
"#;
    let r = upload(
        &c,
        &format!("{base}/api/admin/themes"),
        &token,
        build_zip(&[("bad-dup/theme.toml", dup_key)]),
    )
    .await;
    assert_eq!(r.status(), 422);
    assert_eq!(err_code(r).await, "invalid_manifest");

    let bad_default = r#"name = "坏设置"
slug = "bad-default"
version = "1.0.0"

[[settings]]
key = "a"
type = "switch"
default = "on"
"#;
    let r = upload(
        &c,
        &format!("{base}/api/admin/themes"),
        &token,
        build_zip(&[("bad-default/theme.toml", bad_default)]),
    )
    .await;
    assert_eq!(r.status(), 422);
    assert_eq!(err_code(r).await, "invalid_manifest");
}

// ---------- 3. 按 slug 隔离 + 删除主题清理设置（fixture zip 验证第三方声明解析） ----------

#[tokio::test(flavor = "multi_thread")]
async fn per_theme_isolation_and_delete_cleanup() {
    let tmp = tempfile::tempdir().unwrap();
    let base = spawn_server(tmp.path().join("config.toml").to_str().unwrap()).await;
    let c = reqwest::Client::new();
    let token = setup_installed(&c, &base, tmp.path()).await;

    // default 主题保存自己的设置
    let (status, _) = put_settings(
        &c,
        &base,
        &token,
        "default",
        json!({"layout": "topbar-minimal-three-column", "accent_color": "#112233"}),
    )
    .await;
    assert_eq!(status, 200);

    // 上传 fixture 第三方主题（带 [[settings]] 声明）→ 声明被正确解析下发
    let r = upload(
        &c,
        &format!("{base}/api/admin/themes"),
        &token,
        fixture_zip("my-theme.zip"),
    )
    .await;
    assert_eq!(r.status(), 201);
    let (status, v) = public_settings(&c, &base, "my-theme").await;
    assert_eq!(status, 200);
    let decls = v["settings"].as_array().unwrap();
    assert_eq!(decls.len(), 3);
    let accent = decls.iter().find(|d| d["key"] == "accent_color").unwrap();
    assert_eq!(accent["type"], "color");
    assert_eq!(accent["label"], "强调色");
    assert_eq!(accent["group"], "配色");
    let badge = decls
        .iter()
        .find(|d| d["key"] == "show_footer_badge")
        .unwrap();
    assert_eq!(
        badge["label"], "show_footer_badge",
        "label 缺省应归一化为 key"
    );
    assert_eq!(v["values"]["accent_color"], "#38bdf8"); // 声明默认值
    assert_eq!(v["values"]["show_footer_badge"], true);
    assert!(
        v["values"].get("notice").is_none(),
        "无 default 无存储的 key 不出现"
    );

    // my-theme 保存自己的设置
    let (status, _) = put_settings(
        &c,
        &base,
        &token,
        "my-theme",
        json!({"accent_color": "#123456", "show_footer_badge": false, "notice": "hello"}),
    )
    .await;
    assert_eq!(status, 200);

    // 激活 my-theme：panel 变为 my-theme 的声明与值
    let r = c
        .post(format!("{base}/api/admin/themes/my-theme/activate"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    let p: Value = c
        .get(format!("{base}/api/admin/themes/active/settings-panel"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(p["slug"], "my-theme");
    assert_eq!(p["values"]["accent_color"], "#123456");
    assert_eq!(p["values"]["show_footer_badge"], false);
    assert_eq!(p["values"]["notice"], "hello");

    // 切回 default：panel 回到 default 的设置，先前保存的 layout/配色仍在（隔离不丢）
    let r = c
        .post(format!("{base}/api/admin/themes/default/activate"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    let p: Value = c
        .get(format!("{base}/api/admin/themes/active/settings-panel"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(p["slug"], "default");
    assert_eq!(p["values"]["layout"], "topbar-minimal-three-column");
    assert_eq!(p["values"]["accent_color"], "#112233");

    // 再切 my-theme：它的值也原样保留
    c.post(format!("{base}/api/admin/themes/my-theme/activate"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    let (_, v) = public_settings(&c, &base, "my-theme").await;
    assert_eq!(v["values"]["accent_color"], "#123456");

    // 切回 default 后删除 my-theme → 设置行连带清理；重装后回到声明默认值
    c.post(format!("{base}/api/admin/themes/default/activate"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    let r = c
        .delete(format!("{base}/api/admin/themes/my-theme"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 204);
    // 删除后公开端点 404（磁盘已无该主题）
    let (status, _) = public_settings(&c, &base, "my-theme").await;
    assert_eq!(status, 404);

    let r = upload(
        &c,
        &format!("{base}/api/admin/themes"),
        &token,
        fixture_zip("my-theme.zip"),
    )
    .await;
    assert_eq!(r.status(), 201);
    let (_, v) = public_settings(&c, &base, "my-theme").await;
    assert_eq!(
        v["values"]["accent_color"], "#38bdf8",
        "重装后应回声明默认值"
    );
    assert_eq!(v["values"]["show_footer_badge"], true);
    assert!(v["values"].get("notice").is_none(), "旧存储应已被清理");
}

// ---------- 4. 未安装门禁白名单 ----------

#[tokio::test(flavor = "multi_thread")]
async fn not_installed_whitelist() {
    let tmp = tempfile::tempdir().unwrap();
    let base = spawn_server(tmp.path().join("config.toml").to_str().unwrap()).await;
    let c = reqwest::Client::new();

    // default：未安装也可读（内置常量兜底），values=声明默认值，保证安装页有样式
    let (status, v) = public_settings(&c, &base, "default").await;
    assert_eq!(status, 200);
    assert_eq!(v["slug"], "default");
    assert_eq!(v["values"]["layout"], "topbar-two-column");
    assert_eq!(v["settings"].as_array().unwrap().len(), 3);

    // 其他 slug：磁盘不存在 → 404（不再 503）
    let (status, _) = public_settings(&c, &base, "my-theme").await;
    assert_eq!(status, 404);

    // 管理端点仍被门禁拦截 → 503 not_installed
    let r = c
        .get(format!("{base}/api/admin/themes/active/settings-panel"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 503);
    assert_eq!(err_code(r).await, "not_installed");

    // PUT 非 GET，不在白名单 → 503
    let r = c
        .put(format!("{base}/api/themes/default/settings"))
        .json(&json!({"values": {}}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 503);
    let r = c
        .put(format!("{base}/api/admin/themes/default/settings"))
        .json(&json!({"values": {}}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 503);
}
