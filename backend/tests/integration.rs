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
    assert_eq!(
        r.json::<Value>().await.unwrap(),
        json!({"installed": false})
    );

    // 其余 /api/*（含未定义路径）一律 503 not_installed
    let paths = [
        "/api/site",
        "/api/posts",
        "/api/tags",
        "/api/categories",
        "/api/archive",
        "/api/search",
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
    assert_eq!(
        serde_json::from_str::<Value>(&text).unwrap(),
        json!({"ok": true})
    );

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
    let r = c.get(format!("{base}/api/auth/me")).send().await.unwrap();
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
    // （total = 2 篇测试文章 + 3 篇安装注入的示例文章）
    let v = c
        .get(format!("{base}/api/posts"))
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    assert_eq!(v["total"], 5);
    assert_eq!(v["page"], 1);
    assert_eq!(v["per_page"], 10);
    let items = v["items"].as_array().unwrap();
    assert_eq!(items.len(), 5);
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
    // （total = 2 条测试评论 + 1 条安装注入的示例评论；created_at DESC，示例评论最旧排最后）
    let v = c
        .get(format!("{base}/api/admin/comments"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    assert_eq!(v["total"], 3);
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
    // （列表按 name 排序且含安装注入的示例分类/标签，断言用按名查找而非固定下标）
    let v = c
        .get(format!("{base}/api/categories"))
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    let cats = v.as_array().unwrap();
    let keji = cats
        .iter()
        .find(|x| x["name"] == "科技")
        .expect("应有 科技 分类");
    assert_eq!(keji["post_count"], 1);
    assert!(
        cats.iter().any(|x| x["name"] == "技术分享"),
        "示例分类应在列"
    );
    let v = c
        .get(format!("{base}/api/tags"))
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    let tags = v.as_array().unwrap();
    let rust = tags
        .iter()
        .find(|x| x["name"] == "rust")
        .expect("应有 rust 标签");
    assert_eq!(rust["post_count"], 1);

    // 归档：按年月（3 个示例文章月份 + 测试文章的当前月，共 4 组；年月 DESC，当前月最前）
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
    assert_eq!(arr.len(), 4);
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
    // （published/all 含 3 篇安装注入的示例文章：2 测试 + 3 示例 = 5）
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
    assert_eq!(v["total"], 5);
    let v = c
        .get(format!("{base}/api/admin/posts?status=all"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    assert_eq!(v["total"], 5);
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
    // 删掉 p1 后剩：1 篇测试发布的「Draft One 改题」+ 3 篇安装注入的示例文章
    assert_eq!(v["total"], 4);
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
    // 1 篇实例 1 发布的文章 + 3 篇安装注入的示例文章；重启恢复绝不重复注入
    assert_eq!(v["total"], 4);

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

// ---------- 5. 安装完成自动注入示例数据（契约「安装向导」条款） ----------

#[tokio::test(flavor = "multi_thread")]
async fn install_seeds_sample_content() {
    let tmp = tempfile::tempdir().unwrap();
    let cfg_path = tmp.path().join("config.toml");
    let cfg_str = cfg_path.to_str().unwrap();
    let base = spawn_server(cfg_str).await;
    let c = reqwest::Client::new();

    let r = c
        .post(format!("{base}/api/install"))
        .json(&install_body(tmp.path()))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 201);

    // 3 篇已发布示例文章，发布时间错开在三个不同月份（published_at DESC）
    let v = c
        .get(format!("{base}/api/posts"))
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    assert_eq!(v["total"], 3);
    let items = v["items"].as_array().unwrap();
    assert_eq!(items.len(), 3);
    let months: std::collections::HashSet<String> = items
        .iter()
        .map(|i| i["published_at"].as_str().unwrap()[..7].to_string())
        .collect();
    assert_eq!(months.len(), 3, "示例文章发布时间应错开成不同月份");

    let by_title = |t: &str| -> Value {
        items
            .iter()
            .find(|i| i["title"] == t)
            .unwrap_or_else(|| panic!("缺少示例文章: {t}"))
            .clone()
    };
    let welcome = by_title("欢迎使用 reedblog");
    let axum_post = by_title("用 Axum 和 SQLx 搭建轻量博客后端");
    let essay = by_title("周末随笔：慢下来的时光");

    // slug 生成规则与现有发文流程一致：ASCII slugify；纯中文标题回退 post-<id>
    assert_eq!(welcome["slug"], "reedblog");
    assert_eq!(axum_post["slug"], "axum-sqlx");
    assert_eq!(
        essay["slug"],
        format!("post-{}", essay["id"].as_i64().unwrap())
    );

    // 每篇挂 1 个分类 + 2~3 个标签；excerpt 非空且 ≤200 字符
    for it in items {
        assert!(
            it["category"]["name"].as_str().is_some(),
            "示例文章应挂分类: {it}"
        );
        let n = it["tags"].as_array().unwrap().len();
        assert!((2..=3).contains(&n), "示例文章应挂 2~3 个标签: {it}");
        let excerpt = it["excerpt"].as_str().unwrap();
        assert!(!excerpt.is_empty(), "示例文章 excerpt 不应为空: {it}");
        assert!(excerpt.chars().count() <= 200, "{excerpt}");
    }

    // excerpt 留空的那篇 → 后端自动摘要（剥离 Markdown 的纯文本，无 markup 符号）
    let essay_excerpt = essay["excerpt"].as_str().unwrap();
    for sym in ['#', '*', '`', '>', '|', '[', ']'] {
        assert!(
            !essay_excerpt.contains(sym),
            "自动摘要不应含 Markdown 符号 {sym}: {essay_excerpt}"
        );
    }
    // 显式 excerpt 的那篇原样返回
    assert!(welcome["excerpt"].as_str().unwrap().contains("示例文章"));

    // 详情可读：content_md/content_html 正常渲染
    let v = c
        .get(format!("{base}/api/posts/axum-sqlx"))
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    assert!(v["content_md"].as_str().unwrap().contains("## "));
    assert!(v["content_html"].as_str().unwrap().contains("<h2"));
    assert_eq!(v["comment_count"], 1);

    // 示例分类齐全，post_count 与实际挂载一致（只统计 published）
    let v = c
        .get(format!("{base}/api/categories"))
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    let cats = v.as_array().unwrap();
    assert_eq!(cats.len(), 3);
    for (name, count) in [("技术分享", 1), ("生活随笔", 1), ("默认分类", 1)] {
        let cat = cats
            .iter()
            .find(|x| x["name"] == name)
            .unwrap_or_else(|| panic!("缺少示例分类: {name}"));
        assert_eq!(cat["post_count"], count, "{name}");
    }

    // 示例标签齐全，post_count 与实际挂载一致
    let v = c
        .get(format!("{base}/api/tags"))
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    let tags = v.as_array().unwrap();
    for (name, count) in [
        ("Rust", 1),
        ("前端", 1),
        ("教程", 2),
        ("随笔", 2),
        ("生活", 1),
    ] {
        let tag = tags
            .iter()
            .find(|x| x["name"] == name)
            .unwrap_or_else(|| panic!("缺少示例标签: {name}"));
        assert_eq!(tag["post_count"], count, "{name}");
    }

    // 归档：三篇分布在三个不同年月，各 1 篇
    let v = c
        .get(format!("{base}/api/archive"))
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    let arr = v.as_array().unwrap();
    assert_eq!(arr.len(), 3);
    for e in arr {
        assert_eq!(e["count"], 1, "{e}");
    }

    // 示例访客评论公开可见（先发后审 → approved），CommentPub 形状不含 email/status
    let v = c
        .get(format!("{base}/api/posts/axum-sqlx/comments"))
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    let arr = v.as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert!(!arr[0]["author_name"].as_str().unwrap().is_empty());
    assert!(!arr[0]["content"].as_str().unwrap().is_empty());
    assert!(arr[0]["created_at"].as_str().unwrap().ends_with('Z'));
    assert!(arr[0].get("email").is_none(), "CommentPub 不含 email");
    assert!(arr[0].get("status").is_none(), "CommentPub 不含 status");

    // 后台可管理：示例评论是 approved、挂在示例文章上；示例文章全部 published
    let token = login(&c, &base).await;
    let v = c
        .get(format!("{base}/api/admin/comments?status=approved"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    assert_eq!(v["total"], 1);
    assert_eq!(
        v["items"][0]["post_title"],
        "用 Axum 和 SQLx 搭建轻量博客后端"
    );
    assert!(v["items"][0]["email"].as_str().unwrap().contains('@'));
    let v = c
        .get(format!("{base}/api/admin/posts?status=published"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    assert_eq!(v["total"], 3);

    // 正常启动路径绝不重复注入：模拟重启（startup_state 恢复），示例数据数量不变
    let state2 = reedblog_backend::startup_state(cfg_str).await;
    assert!(state2.is_installed().await);
    let app2 = reedblog_backend::build_router(state2, vec!["http://localhost:5173".to_string()]);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base2 = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
    tokio::spawn(async move {
        axum::serve(listener, app2).await.unwrap();
    });
    let v = c
        .get(format!("{base2}/api/posts"))
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    assert_eq!(v["total"], 3, "重启恢复不得重复注入示例文章");
    let v = c
        .get(format!("{base2}/api/categories"))
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    assert_eq!(
        v.as_array().unwrap().len(),
        3,
        "重启恢复不得重复注入示例分类"
    );
    let v = c
        .get(format!("{base2}/api/posts/axum-sqlx/comments"))
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    assert_eq!(
        v.as_array().unwrap().len(),
        1,
        "重启恢复不得重复注入示例评论"
    );
}
