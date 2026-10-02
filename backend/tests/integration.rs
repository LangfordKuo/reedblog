//! 集成测试：真实 HTTP 端口 + SQLite 临时库。
//! 覆盖验收链路：安装 → 登录 → 发文 → 公开读取 → 评论，以及未安装门禁、重启恢复。

use reedblog_backend::state::AppState;
use serde_json::{json, Value};
use std::path::Path;

/// 在 127.0.0.1 随机端口起一个真实服务，返回 base URL。
/// 配置文件不存在时预写 [plugins]/[themes] dir 指向同目录（测试隔离，
/// 避免安装时把内置 default 主题生成到仓库工作目录）；安装流程会保留这两段。
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

fn install_body(dir: &Path) -> Value {
    json!({
        "db_type": "sqlite",
        "sqlite_path": dir.join("reedblog.db").to_str().unwrap(),
        "admin": {"username": "admin", "password": "secret123"},
        "site": {"title": "测试博客", "subtitle": "副标题"}
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
    login(c, base).await
}

async fn login(c: &reqwest::Client, base: &str) -> String {
    let r = c
        .post(format!("{base}/api/auth/login"))
        .json(&json!({"username": "admin", "password": "secret123"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    let v: Value = r.json().await.unwrap();
    v["token"].as_str().unwrap().to_string()
}

// ---------- 1. 未安装门禁 ----------

#[tokio::test(flavor = "multi_thread")]
async fn not_installed_gate() {
    let tmp = tempfile::tempdir().unwrap();
    let base = spawn_server(tmp.path().join("config.toml").to_str().unwrap()).await;
    let c = reqwest::Client::new();

    // health 永远可用
    let r = c.get(format!("{base}/api/health")).send().await.unwrap();
    assert_eq!(r.status(), 200);
    assert_eq!(r.json::<Value>().await.unwrap(), json!({"status": "ok"}));

    // install/status 永远可用
    let r = c
        .get(format!("{base}/api/install/status"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    assert_eq!(r.json::<Value>().await.unwrap(), json!({"installed": false}));

    // 其余 /api/*（含未定义路径）一律 503 not_installed
    let paths = [
        "/api/site",
        "/api/posts",
        "/api/tags",
        "/api/categories",
        "/api/archive",
        "/api/auth/me",
        "/api/admin/posts",
        "/api/admin/comments",
        "/api/no-such-route",
    ];
    for p in paths {
        let r = c.get(format!("{base}{p}")).send().await.unwrap();
        assert_eq!(r.status(), 503, "GET {p} 未安装时应 503");
        assert_eq!(err_code(r).await, "not_installed", "GET {p}");
    }
    // 写操作同样被挡
    let r = c
        .post(format!("{base}/api/admin/posts"))
        .json(&json!({"title": "x", "content_md": "y", "status": "draft"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 503);
    assert_eq!(err_code(r).await, "not_installed");
    // 未安装时登录也 503
    let r = c
        .post(format!("{base}/api/auth/login"))
        .json(&json!({"username": "a", "password": "b"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 503);

    // 安装请求体校验（缺 admin）→ 422 validation_error
    let r = c
        .post(format!("{base}/api/install"))
        .json(&json!({"db_type": "sqlite", "site": {"title": "t"}}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 422);
    assert_eq!(err_code(r).await, "validation_error");
}

// ---------- 2. 主验收链路：安装→登录→发文→公开读取→评论 ----------

#[tokio::test(flavor = "multi_thread")]
async fn install_login_publish_comment_flow() {
    let tmp = tempfile::tempdir().unwrap();
    let cfg_path = tmp.path().join("config.toml");
    let base = spawn_server(cfg_path.to_str().unwrap()).await;
    let c = reqwest::Client::new();

    // 安装 → 201 {"ok": true}
    let r = c
        .post(format!("{base}/api/install"))
        .json(&install_body(tmp.path()))
        .send()
        .await
        .unwrap();
    let status = r.status();
    let text = r.text().await.unwrap();
    assert_eq!(status, 201, "install 失败: {text}");
    assert_eq!(serde_json::from_str::<Value>(&text).unwrap(), json!({"ok": true}));

    // config.toml 已写入且含 jwt_secret
    let cfg_text = std::fs::read_to_string(&cfg_path).unwrap();
    assert!(cfg_text.contains("jwt_secret"));
    assert!(cfg_text.contains("测试博客"));

    // 重复安装 → 409 already_installed
    let r = c
        .post(format!("{base}/api/install"))
        .json(&install_body(tmp.path()))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 409);
    assert_eq!(err_code(r).await, "already_installed");

    // install/status → true；/api/site 可用
    let v = c
        .get(format!("{base}/api/install/status"))
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    assert_eq!(v, json!({"installed": true}));
    let v = c
        .get(format!("{base}/api/site"))
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    assert_eq!(v["title"], "测试博客");
    assert_eq!(v["subtitle"], "副标题");
    assert_eq!(v["installed"], true);

    // 登录：密码错误 → 401 invalid_credentials
    let r = c
        .post(format!("{base}/api/auth/login"))
        .json(&json!({"username": "admin", "password": "wrong-pass"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 401);
    assert_eq!(err_code(r).await, "invalid_credentials");

    // 登录成功 → AuthResult
    let r = c
        .post(format!("{base}/api/auth/login"))
        .json(&json!({"username": "admin", "password": "secret123"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    let v: Value = r.json().await.unwrap();
    let token = v["token"].as_str().unwrap().to_string();
    assert!(!token.is_empty());
    assert_eq!(v["username"], "admin");
    assert!(v["expires_at"].as_str().unwrap().ends_with('Z'));

    // /api/auth/me：无 token / 坏 token → 401 unauthorized；好 token → {"username"}
    let r = c
        .get(format!("{base}/api/auth/me"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 401);
    assert_eq!(err_code(r).await, "unauthorized");
    let r = c
        .get(format!("{base}/api/auth/me"))
        .bearer_auth("bad.token.value")
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 401);
    let v = c
        .get(format!("{base}/api/auth/me"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    assert_eq!(v, json!({"username": "admin"}));

    // 管理接口无 token → 401
    let r = c
        .post(format!("{base}/api/admin/posts"))
        .json(&json!({"title": "x", "content_md": "y", "status": "draft"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 401);

    // 发文：英文标题 → ASCII slugify
    let r = c
        .post(format!("{base}/api/admin/posts"))
        .bearer_auth(&token)
        .json(&json!({
            "title": "Hello World", "content_md": "# Hi there",
            "excerpt": "摘要A", "status": "published"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 201);
    let p1: Value = r.json().await.unwrap();
    assert_eq!(p1["slug"], "hello-world");
    assert_eq!(p1["status"], "published");
    assert!(p1["published_at"].as_str().is_some());
    assert_eq!(p1["excerpt"], "摘要A");

    // 发文：纯中文标题 → 回退 post-<id>
    let r = c
        .post(format!("{base}/api/admin/posts"))
        .bearer_auth(&token)
        .json(&json!({"title": "你好，世界", "content_md": "中文正文", "status": "published"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 201);
    let p2: Value = r.json().await.unwrap();
    let id2 = p2["id"].as_i64().unwrap();
    assert_eq!(p2["slug"], format!("post-{id2}"));

    // 发文：草稿无 published_at；excerpt 缺省自动从正文推导
    let r = c
        .post(format!("{base}/api/admin/posts"))
        .bearer_auth(&token)
        .json(&json!({"title": "My Draft", "content_md": "draft body", "status": "draft"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 201);
    let p3: Value = r.json().await.unwrap();
    assert_eq!(p3["slug"], "my-draft");
    assert!(p3["published_at"].is_null());
    assert_eq!(p3["excerpt"], "draft body");

    // slug 冲突 → 409 slug_taken
    let r = c
        .post(format!("{base}/api/admin/posts"))
        .bearer_auth(&token)
        .json(&json!({"title": "Another", "slug": "hello-world", "content_md": "x", "status": "draft"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 409);
    assert_eq!(err_code(r).await, "slug_taken");

    // 缺必填 title → 422 validation_error
    let r = c
        .post(format!("{base}/api/admin/posts"))
        .bearer_auth(&token)
        .json(&json!({"content_md": "x", "status": "draft"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 422);
    assert_eq!(err_code(r).await, "validation_error");

    // 公开列表：仅 published、分页形状、列表不含 content_md
    let v = c
        .get(format!("{base}/api/posts"))
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    assert_eq!(v["total"], 2);
    assert_eq!(v["page"], 1);
    assert_eq!(v["per_page"], 10);
    let items = v["items"].as_array().unwrap();
    assert_eq!(items.len(), 2);
    for it in items {
        assert!(it.get("content_md").is_none(), "公开列表不含 content_md");
        assert!(it.get("comment_count").is_some());
    }

    // per_page 上限 100
    let v = c
        .get(format!("{base}/api/posts?per_page=500"))
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    assert_eq!(v["per_page"], 100);

    // 详情：published 可读，draft 404
    let v = c
        .get(format!("{base}/api/posts/hello-world"))
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    assert_eq!(v["title"], "Hello World");
    assert_eq!(v["content_md"], "# Hi there");
    let r = c
        .get(format!("{base}/api/posts/my-draft"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 404);
    assert_eq!(err_code(r).await, "not_found");

    // 评论：缺必填 → 422 validation_error
    let r = c
        .post(format!("{base}/api/posts/hello-world/comments"))
        .json(&json!({"content": "没有名字"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 422);
    assert_eq!(err_code(r).await, "validation_error");
    let r = c
        .post(format!("{base}/api/posts/hello-world/comments"))
        .json(&json!({"author_name": "   ", "content": "x"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 422);

    // 评论：不存在/未发布文章 → 404
    let r = c
        .post(format!("{base}/api/posts/no-such-post/comments"))
        .json(&json!({"author_name": "x", "content": "y"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 404);

    // 评论：先发后审，创建即 approved → 201 CommentPub
    let r = c
        .post(format!("{base}/api/posts/hello-world/comments"))
        .json(&json!({"author_name": "读者甲", "email": "a@example.com", "content": "好文！"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 201);
    let cm1: Value = r.json().await.unwrap();
    assert_eq!(cm1["author_name"], "读者甲");
    assert_eq!(cm1["content"], "好文！");
    assert!(cm1["created_at"].as_str().unwrap().ends_with('Z'));
    assert!(cm1.get("email").is_none(), "CommentPub 不含 email");
    assert!(cm1.get("status").is_none(), "CommentPub 不含 status");
    let cm1_id = cm1["id"].as_i64().unwrap();

    let r = c
        .post(format!("{base}/api/posts/hello-world/comments"))
        .json(&json!({"author_name": "读者乙", "content": "第二条评论"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 201);
    let cm2_id = r.json::<Value>().await.unwrap()["id"].as_i64().unwrap();

    // 公开评论列表：approved、时间 ASC
    let v = c
        .get(format!("{base}/api/posts/hello-world/comments"))
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    let arr = v.as_array().unwrap();
    assert_eq!(arr.len(), 2);
    assert_eq!(arr[0]["author_name"], "读者甲");
    assert_eq!(arr[1]["author_name"], "读者乙");

    // 详情 comment_count 同步更新
    let v = c
        .get(format!("{base}/api/posts/hello-world"))
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    assert_eq!(v["comment_count"], 2);

    // 管理端评论：列表（含 post_title/email）→ 隐藏 → 公开不可见 → 删除
    let v = c
        .get(format!("{base}/api/admin/comments"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    assert_eq!(v["total"], 2);
    assert_eq!(v["items"][0]["post_title"], "Hello World");

    let r = c
        .put(format!("{base}/api/admin/comments/{cm1_id}"))
        .bearer_auth(&token)
        .json(&json!({"status": "hidden"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    assert_eq!(r.json::<Value>().await.unwrap()["status"], "hidden");

    // 非法 status → 422
    let r = c
        .put(format!("{base}/api/admin/comments/{cm1_id}"))
        .bearer_auth(&token)
        .json(&json!({"status": "deleted"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 422);
    assert_eq!(err_code(r).await, "validation_error");

    // 公开列表只剩 approved 的读者乙
    let v = c
        .get(format!("{base}/api/posts/hello-world/comments"))
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    let arr = v.as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["author_name"], "读者乙");

    // status 过滤
    let v = c
        .get(format!("{base}/api/admin/comments?status=hidden"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    assert_eq!(v["total"], 1);
    assert_eq!(v["items"][0]["id"], cm1_id);

    // post_id 过滤（两条评论都挂在 p1 上：1 hidden + 1 approved）
    let p1_id = p1["id"].as_i64().unwrap();
    let v = c
        .get(format!("{base}/api/admin/comments?post_id={p1_id}"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    assert_eq!(v["total"], 2);
    // post_id + status 组合过滤
    let v = c
        .get(format!(
            "{base}/api/admin/comments?post_id={p1_id}&status=hidden"
        ))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    assert_eq!(v["total"], 1);
    assert_eq!(v["items"][0]["id"], cm1_id);
    let v = c
        .get(format!("{base}/api/admin/comments?post_id=99999"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    assert_eq!(v["total"], 0);

    // 删除 → 204；再删 → 404
    let r = c
        .delete(format!("{base}/api/admin/comments/{cm2_id}"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 204);
    let r = c
        .delete(format!("{base}/api/admin/comments/{cm2_id}"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 404);

    // 安装后未定义路径 → 404（不再是 503）
    let r = c
        .get(format!("{base}/api/no-such-route"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 404);
}

// ---------- 3. 分类/标签/归档 + 管理端文章 CRUD ----------

#[tokio::test(flavor = "multi_thread")]
async fn taxonomies_archive_and_admin_crud() {
    let tmp = tempfile::tempdir().unwrap();
    let base = spawn_server(tmp.path().join("config.toml").to_str().unwrap()).await;
    let c = reqwest::Client::new();
    let token = setup_installed(&c, &base, tmp.path()).await;

    // 分类/标签创建
    let r = c
        .post(format!("{base}/api/admin/categories"))
        .bearer_auth(&token)
        .json(&json!({"name": "技术"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 201);
    let cat: Value = r.json().await.unwrap();
    let cat_id = cat["id"].as_i64().unwrap();
    assert_eq!(cat["post_count"], 0);

    // 重名 → 409 duplicate_name
    let r = c
        .post(format!("{base}/api/admin/categories"))
        .bearer_auth(&token)
        .json(&json!({"name": "技术"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 409);
    assert_eq!(err_code(r).await, "duplicate_name");

    let r = c
        .post(format!("{base}/api/admin/tags"))
        .bearer_auth(&token)
        .json(&json!({"name": "rust"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 201);
    let tag_id = r.json::<Value>().await.unwrap()["id"].as_i64().unwrap();
    let r = c
        .post(format!("{base}/api/admin/tags"))
        .bearer_auth(&token)
        .json(&json!({"name": "rust"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 409);
    assert_eq!(err_code(r).await, "duplicate_name");

    // 改名
    let r = c
        .put(format!("{base}/api/admin/categories/{cat_id}"))
        .bearer_auth(&token)
        .json(&json!({"name": "科技"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    assert_eq!(r.json::<Value>().await.unwrap()["name"], "科技");

    // 带分类+标签发文
    let r = c
        .post(format!("{base}/api/admin/posts"))
        .bearer_auth(&token)
        .json(&json!({
            "title": "Rust 入门", "content_md": "content", "status": "published",
            "category_id": cat_id, "tag_ids": [tag_id]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 201);
    let p1: Value = r.json().await.unwrap();
    let p1_id = p1["id"].as_i64().unwrap();
    assert_eq!(p1["category_id"], cat_id);
    assert_eq!(p1["category_name"], "科技");
    assert_eq!(p1["tag_ids"], json!([tag_id]));

    // 不存在的 category_id → 422
    let r = c
        .post(format!("{base}/api/admin/posts"))
        .bearer_auth(&token)
        .json(&json!({"title": "x", "content_md": "y", "status": "draft", "category_id": 9999}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 422);

    // 公开过滤：category / tag
    let v = c
        .get(format!("{base}/api/posts?category=科技"))
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    assert_eq!(v["total"], 1);
    let v = c
        .get(format!("{base}/api/posts?category=技术"))
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    assert_eq!(v["total"], 0);
    let v = c
        .get(format!("{base}/api/posts?tag=rust"))
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    assert_eq!(v["total"], 1);
    assert_eq!(v["items"][0]["category"]["name"], "科技");
    assert_eq!(v["items"][0]["tags"][0]["name"], "rust");

    // 公开分类/标签 post_count 只统计 published
    let v = c
        .get(format!("{base}/api/categories"))
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    assert_eq!(v[0]["name"], "科技");
    assert_eq!(v[0]["post_count"], 1);
    let v = c
        .get(format!("{base}/api/tags"))
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    assert_eq!(v[0]["post_count"], 1);

    // 归档：按年月
    let now = chrono::Utc::now();
    let v = c
        .get(format!("{base}/api/archive"))
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    let arr = v.as_array().unwrap();
    assert_eq!(arr.len(), 1);
    let year = now.format("%Y").to_string().parse::<i64>().unwrap();
    let month = now.format("%m").to_string().parse::<i64>().unwrap();
    assert_eq!(arr[0]["year"], year);
    assert_eq!(arr[0]["month"], month);
    assert_eq!(arr[0]["count"], 1);

    // 公开列表 year/month 归档过滤 + 不存在的 tag 过滤
    let v = c
        .get(format!("{base}/api/posts?year={year}&month={month}"))
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    assert_eq!(v["total"], 1);
    let v = c
        .get(format!("{base}/api/posts?year=2001"))
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    assert_eq!(v["total"], 0);
    let v = c
        .get(format!("{base}/api/posts?tag=no-such-tag"))
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    assert_eq!(v["total"], 0);

    // in_use：分类/标签被引用时不可删
    let r = c
        .delete(format!("{base}/api/admin/categories/{cat_id}"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 409);
    assert_eq!(err_code(r).await, "in_use");
    let r = c
        .delete(format!("{base}/api/admin/tags/{tag_id}"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 409);
    assert_eq!(err_code(r).await, "in_use");

    // 草稿 → draft→published 补写 published_at
    let r = c
        .post(format!("{base}/api/admin/posts"))
        .bearer_auth(&token)
        .json(&json!({"title": "Draft One", "content_md": "d", "status": "draft"}))
        .send()
        .await
        .unwrap();
    let d1: Value = r.json().await.unwrap();
    let d1_id = d1["id"].as_i64().unwrap();
    assert!(d1["published_at"].is_null());

    let r = c
        .put(format!("{base}/api/admin/posts/{d1_id}"))
        .bearer_auth(&token)
        .json(&json!({"status": "published", "title": "Draft One 改题"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    let upd: Value = r.json().await.unwrap();
    assert_eq!(upd["status"], "published");
    assert_eq!(upd["title"], "Draft One 改题");
    assert!(upd["published_at"].as_str().is_some());

    // 管理列表过滤：status=draft/published/all + 非法值
    let v = c
        .get(format!("{base}/api/admin/posts?status=draft"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    assert_eq!(v["total"], 0);
    let v = c
        .get(format!("{base}/api/admin/posts?status=published"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    assert_eq!(v["total"], 2);
    let v = c
        .get(format!("{base}/api/admin/posts?status=all"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    assert_eq!(v["total"], 2);
    let r = c
        .get(format!("{base}/api/admin/posts?status=bogus"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 422);

    // 单篇读取 + 404
    let v = c
        .get(format!("{base}/api/admin/posts/{p1_id}"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    assert_eq!(v["title"], "Rust 入门");
    let r = c
        .get(format!("{base}/api/admin/posts/99999"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 404);

    // 显式 null 清空分类
    let r = c
        .put(format!("{base}/api/admin/posts/{p1_id}"))
        .bearer_auth(&token)
        .json(&json!({"category_id": null}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    let upd: Value = r.json().await.unwrap();
    assert!(upd["category_id"].is_null());
    assert!(upd["category_name"].is_null());

    // 清空标签后，标签/分类可删
    let r = c
        .put(format!("{base}/api/admin/posts/{p1_id}"))
        .bearer_auth(&token)
        .json(&json!({"tag_ids": []}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.json::<Value>().await.unwrap()["tag_ids"], json!([]));
    let r = c
        .delete(format!("{base}/api/admin/tags/{tag_id}"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 204);
    let r = c
        .delete(format!("{base}/api/admin/categories/{cat_id}"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 204);

    // 删除文章 → 204；公开列表减少；再读 404
    let r = c
        .delete(format!("{base}/api/admin/posts/{p1_id}"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 204);
    let r = c
        .get(format!("{base}/api/admin/posts/{p1_id}"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 404);
    let v = c
        .get(format!("{base}/api/posts"))
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    assert_eq!(v["total"], 1);
}

// ---------- 4. 重启恢复：同一 config.toml + SQLite 文件，新实例即已安装 ----------

#[tokio::test(flavor = "multi_thread")]
async fn startup_restores_installed_state() {
    let tmp = tempfile::tempdir().unwrap();
    let cfg_path = tmp.path().join("config.toml");
    let cfg_str = cfg_path.to_str().unwrap();
    let c = reqwest::Client::new();

    // 实例 1：安装 + 发文
    let base1 = spawn_server(cfg_str).await;
    let token1 = setup_installed(&c, &base1, tmp.path()).await;
    let r = c
        .post(format!("{base1}/api/admin/posts"))
        .bearer_auth(&token1)
        .json(&json!({"title": "Persisted Post", "content_md": "body", "status": "published"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 201);

    // 实例 2：模拟进程重启（同一 config.toml，走 startup_state 恢复）
    let state2 = reedblog_backend::startup_state(cfg_str).await;
    assert!(state2.is_installed().await);
    let app2 = reedblog_backend::build_router(state2, vec!["http://localhost:5173".to_string()]);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base2 = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
    tokio::spawn(async move {
        axum::serve(listener, app2).await.unwrap();
    });

    // 已安装状态与数据都在
    let v = c
        .get(format!("{base2}/api/install/status"))
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    assert_eq!(v, json!({"installed": true}));
    let v = c
        .get(format!("{base2}/api/site"))
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    assert_eq!(v["title"], "测试博客");
    let v = c
        .get(format!("{base2}/api/posts"))
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    assert_eq!(v["total"], 1);

    // JWT secret 持久化：实例 1 签发的 token 在实例 2 依然有效
    let v = c
        .get(format!("{base2}/api/auth/me"))
        .bearer_auth(&token1)
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    assert_eq!(v, json!({"username": "admin"}));

    // 重新登录也可以
    let r = c
        .post(format!("{base2}/api/auth/login"))
        .json(&json!({"username": "admin", "password": "secret123"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
}
