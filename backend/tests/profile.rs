//! 管理员资料 / 用户设置集成测试（契约「管理员资料 / 用户设置」条款，2026-10-04 新增）：
//! - profile 接口需要 Bearer（无 token → 401 unauthorized；未安装 → 503 not_installed）
//! - GET 返回当前用户名与创建时间
//! - 改用户名：新用户名可登录、旧用户名登录失败；**已签发的旧 token 仍有效**（JWT 不失效）
//! - 改密码：新密码可登录、旧密码 401；旧 token 仍有效
//! - current_password 错误 → 401 invalid_credentials 且**不落库**（用户名/密码都未变）
//! - 唯一性冲突 → 409 username_taken（直接在库里造第二个用户构造冲突）
//! - 校验：只给 current_password → 422；空用户名/空新密码 → 422；缺 current_password → 422

use reedblog_backend::state::AppState;
use serde_json::{json, Value};
use std::path::Path;

/// 在 127.0.0.1 随机端口起真实服务（与 pages.rs / integration.rs 同款隔离）
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

/// 提取契约错误形状的 code
async fn err_code(r: reqwest::Response) -> String {
    let v: Value = r.json().await.unwrap();
    v["error"]["code"].as_str().unwrap_or_default().to_string()
}

/// 安装 + 登录，返回 (client, base, token)
async fn setup(dir: &Path) -> (reqwest::Client, String, String) {
    let base = spawn_server(dir.join("config.toml").to_str().unwrap()).await;
    let c = reqwest::Client::new();
    let r = c
        .post(format!("{base}/api/install"))
        .json(&json!({
            "db_type": "sqlite",
            "sqlite_path": dir.join("reedblog.db").to_str().unwrap(),
            "admin": {"username": "admin", "password": "secret123"},
            "site": {"title": "用户设置测试站"}
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

/// 登录尝试 → (HTTP 状态码, 契约错误码或空串)
async fn try_login(
    c: &reqwest::Client,
    base: &str,
    username: &str,
    password: &str,
) -> (u16, String) {
    let r = c
        .post(format!("{base}/api/auth/login"))
        .json(&json!({"username": username, "password": password}))
        .send()
        .await
        .unwrap();
    let status = r.status().as_u16();
    if status == 200 {
        (status, String::new())
    } else {
        (status, err_code(r).await)
    }
}

/// 带 Bearer 的 GET /api/admin/profile → (状态码, JSON)
async fn get_profile(c: &reqwest::Client, base: &str, token: &str) -> (u16, Value) {
    let r = c
        .get(format!("{base}/api/admin/profile"))
        .bearer_auth(token)
        .send()
        .await
        .unwrap();
    let status = r.status().as_u16();
    (status, r.json::<Value>().await.unwrap())
}

/// PUT /api/admin/profile → (状态码, JSON)
async fn put_profile(c: &reqwest::Client, base: &str, token: &str, body: Value) -> (u16, Value) {
    let r = c
        .put(format!("{base}/api/admin/profile"))
        .bearer_auth(token)
        .json(&body)
        .send()
        .await
        .unwrap();
    let status = r.status().as_u16();
    (status, r.json::<Value>().await.unwrap())
}

/// 安装写入的 config.toml → SQLite 连接 URL（复用 Config::db_url 的路径编码逻辑）
fn sqlite_url_from_config(config_path: &str) -> String {
    reedblog_backend::config::Config::load(Path::new(config_path))
        .expect("config.toml 应已由安装流程写入")
        .db_url()
        .expect("db_type 应为 sqlite")
}

/// 直接造第二个用户（users.username 唯一）构造改名冲突场景
async fn db_insert_user(url: &str, username: &str) {
    sqlx::any::install_default_drivers();
    let pool = sqlx::AnyPool::connect(url).await.unwrap();
    sqlx::query("INSERT INTO users (username, password_hash, created_at) VALUES (?, ?, ?)")
        .bind(username)
        .bind("not-a-real-hash")
        .bind("2026-01-01T00:00:00Z")
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;
}

// ---------- 1. 鉴权与未安装门禁 ----------

#[tokio::test(flavor = "multi_thread")]
async fn profile_requires_bearer_and_install() {
    // 未安装：不在门禁白名单内 → 503 not_installed
    let tmp = tempfile::tempdir().unwrap();
    let base = spawn_server(tmp.path().join("config.toml").to_str().unwrap()).await;
    let c = reqwest::Client::new();
    let r = c
        .get(format!("{base}/api/admin/profile"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 503);
    assert_eq!(err_code(r).await, "not_installed");

    // 已安装但无 token → 401 unauthorized（GET 与 PUT 一致）
    let tmp = tempfile::tempdir().unwrap();
    let (c, base, _token) = setup(tmp.path()).await;
    let r = c
        .get(format!("{base}/api/admin/profile"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 401);
    assert_eq!(err_code(r).await, "unauthorized");

    let r = c
        .put(format!("{base}/api/admin/profile"))
        .json(&json!({"current_password": "secret123", "username": "hacker"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 401);
    assert_eq!(err_code(r).await, "unauthorized");
}

// ---------- 2. GET 形状 ----------

#[tokio::test(flavor = "multi_thread")]
async fn profile_get_returns_username_and_created_at() {
    let tmp = tempfile::tempdir().unwrap();
    let (c, base, token) = setup(tmp.path()).await;

    let (status, body) = get_profile(&c, &base, &token).await;
    assert_eq!(status, 200);
    assert_eq!(body["username"], json!("admin"));
    let created = body["created_at"].as_str().unwrap();
    assert!(
        created.len() >= 20,
        "created_at 应为 RFC3339 文本: {created}"
    );
    assert!(created.ends_with('Z') || created.contains('+'), "{created}");

    // 响应绝不包含密码哈希字段（契约形状只有 username/created_at）
    let obj = body.as_object().unwrap();
    assert_eq!(
        obj.len(),
        2,
        "ProfileAdmin 只应有 username/created_at: {body}"
    );
}

// ---------- 3. 改用户名：登录切换 + 旧 token 不失效 ----------

#[tokio::test(flavor = "multi_thread")]
async fn rename_username_switches_login_and_keeps_old_token() {
    let tmp = tempfile::tempdir().unwrap();
    let (c, base, token) = setup(tmp.path()).await;

    let (status, body) = put_profile(
        &c,
        &base,
        &token,
        json!({"current_password": "secret123", "username": "  newadmin  "}),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["username"], json!("newadmin"), "用户名应 trim 后写入");

    // 新用户名 + 原密码可登录
    let (status, code) = try_login(&c, &base, "newadmin", "secret123").await;
    assert_eq!((status, code.as_str()), (200, ""), "新用户名应可登录");
    // 旧用户名登录失败（401 invalid_credentials）
    let (status, code) = try_login(&c, &base, "admin", "secret123").await;
    assert_eq!((status, code.as_str()), (401, "invalid_credentials"));

    // 改名前签发的 token 仍有效（契约：JWT 不失效），且 GET 显示新用户名
    let (status, body) = get_profile(&c, &base, &token).await;
    assert_eq!(status, 200, "旧 token 必须继续有效: {body}");
    assert_eq!(
        body["username"],
        json!("newadmin"),
        "profile 应返回当前用户名"
    );
}

// ---------- 4. 唯一性冲突 409（含同名 no-op） ----------

#[tokio::test(flavor = "multi_thread")]
async fn rename_to_taken_username_409() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    let (c, base, token) = setup(dir).await;
    let url = sqlite_url_from_config(dir.join("config.toml").to_str().unwrap());
    db_insert_user(&url, "occupied").await;

    // 改成与现有用户名相同：视为无变化，200（不是冲突）
    let (status, body) = put_profile(
        &c,
        &base,
        &token,
        json!({"current_password": "secret123", "username": "admin"}),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["username"], json!("admin"));

    // 改成已占用名 → 409 username_taken，且不落库
    let (status, body) = put_profile(
        &c,
        &base,
        &token,
        json!({"current_password": "secret123", "username": "occupied"}),
    )
    .await;
    assert_eq!(status, 409, "{body}");
    assert_eq!(body["error"]["code"], json!("username_taken"));

    let (status, body) = get_profile(&c, &base, &token).await;
    assert_eq!(status, 200);
    assert_eq!(body["username"], json!("admin"), "冲突后用户名不应改变");
    let (status, _) = try_login(&c, &base, "occupied", "secret123").await;
    assert_eq!(status, 401, "第二个用户密码无效，登录应失败");
}

// ---------- 5. 改密码：新密码可登录、旧密码 401、旧 token 仍有效 ----------

#[tokio::test(flavor = "multi_thread")]
async fn change_password_switches_login_and_keeps_old_token() {
    let tmp = tempfile::tempdir().unwrap();
    let (c, base, token) = setup(tmp.path()).await;

    let (status, body) = put_profile(
        &c,
        &base,
        &token,
        json!({"current_password": "secret123", "new_password": "brand-new-pass"}),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["username"], json!("admin"));

    let (status, code) = try_login(&c, &base, "admin", "brand-new-pass").await;
    assert_eq!((status, code.as_str()), (200, ""), "新密码应可登录");
    let (status, code) = try_login(&c, &base, "admin", "secret123").await;
    assert_eq!(
        (status, code.as_str()),
        (401, "invalid_credentials"),
        "旧密码应失效"
    );

    // 改密码不使已签发 JWT 失效（契约明确：其它已登录设备仍在线）
    let (status, body) = get_profile(&c, &base, &token).await;
    assert_eq!(status, 200, "改密码后旧 token 必须仍有效: {body}");
}

// ---------- 6. current_password 错误：401 且不落库 ----------

#[tokio::test(flavor = "multi_thread")]
async fn wrong_current_password_401_and_no_write() {
    let tmp = tempfile::tempdir().unwrap();
    let (c, base, token) = setup(tmp.path()).await;

    let (status, body) = put_profile(
        &c,
        &base,
        &token,
        json!({
            "current_password": "wrong-pass",
            "username": "should-not-apply",
            "new_password": "should-not-apply"
        }),
    )
    .await;
    assert_eq!(status, 401, "{body}");
    assert_eq!(body["error"]["code"], json!("invalid_credentials"));

    // 用户名未变
    let (status, body) = get_profile(&c, &base, &token).await;
    assert_eq!(status, 200);
    assert_eq!(body["username"], json!("admin"));
    // 原密码仍可登录、新密码无效（密码也未落库）
    let (status, _) = try_login(&c, &base, "admin", "secret123").await;
    assert_eq!(status, 200);
    let (status, _) = try_login(&c, &base, "admin", "should-not-apply").await;
    assert_eq!(status, 401);
    let (status, _) = try_login(&c, &base, "should-not-apply", "secret123").await;
    assert_eq!(status, 401, "错误密码请求里的用户名不应被写入");
}

// ---------- 7. 校验 422 ----------

#[tokio::test(flavor = "multi_thread")]
async fn profile_validation_422() {
    let tmp = tempfile::tempdir().unwrap();
    let (c, base, token) = setup(tmp.path()).await;

    // 只给 current_password（username/new_password 都不给）→ 422
    let (status, body) =
        put_profile(&c, &base, &token, json!({"current_password": "secret123"})).await;
    assert_eq!(status, 422, "{body}");
    assert_eq!(body["error"]["code"], json!("validation_error"));

    // 显式 null 等同缺失 → 422
    let (status, body) = put_profile(
        &c,
        &base,
        &token,
        json!({"current_password": "secret123", "username": null, "new_password": null}),
    )
    .await;
    assert_eq!(status, 422, "{body}");

    // current_password 字段整体缺失 → 422（请求体形状不合法）
    let (status, body) = put_profile(&c, &base, &token, json!({"username": "someone"})).await;
    assert_eq!(status, 422, "{body}");
    assert_eq!(body["error"]["code"], json!("validation_error"));

    // 空/空白用户名 → 422（与安装向导同款：trim 后非空）
    let (status, body) = put_profile(
        &c,
        &base,
        &token,
        json!({"current_password": "secret123", "username": "   "}),
    )
    .await;
    assert_eq!(status, 422, "{body}");

    // 空新密码 → 422
    let (status, body) = put_profile(
        &c,
        &base,
        &token,
        json!({"current_password": "secret123", "new_password": ""}),
    )
    .await;
    assert_eq!(status, 422, "{body}");

    // 校验失败后账号原样：用户名/密码都未变
    let (status, body) = get_profile(&c, &base, &token).await;
    assert_eq!(status, 200);
    assert_eq!(body["username"], json!("admin"));
    let (status, _) = try_login(&c, &base, "admin", "secret123").await;
    assert_eq!(status, 200);
}
