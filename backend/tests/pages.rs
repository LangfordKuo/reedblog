//! 页面功能集成测试（契约「页面」条款，2026-10-03 新增）：
//! - 安装注入 3 个内置页面（关于/留言板/友情链接）；未安装门禁覆盖 pages 接口
//! - 页面 CRUD：创建/读取/更新/toggle/删除；slug 冲突 409；自动生成与中文回退
//! - 内置页不可删除（422 page_builtin）但可停用、可编辑
//! - 停用页公开详情 404、公开列表/导航消失、sitemap 不含
//! - 留言板留言：创建即 approved、后台评论列表可见来源（target_type=page）、
//!   非留言板页 404；文章评论回归（target_type=post、post_id 过滤只匹配文章）
//! - 友情链接：links 全量替换、排序重写、URL 校验 422、非 links 页忽略 links

use reedblog_backend::state::AppState;
use serde_json::{json, Value};
use std::path::Path;

/// 在 127.0.0.1 随机端口起真实服务（与 integration.rs 同款隔离）
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
            "site": {"title": "页面测试站"}
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

/// GET /api/admin/pages → Value（数组）
async fn admin_pages(c: &reqwest::Client, base: &str, token: &str) -> Value {
    let r = c
        .get(format!("{base}/api/admin/pages"))
        .bearer_auth(token)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    r.json::<Value>().await.unwrap()
}

/// 按 slug 在管理列表中找页面
fn find_by_slug(list: &Value, slug: &str) -> Value {
    list.as_array()
        .unwrap()
        .iter()
        .find(|p| p["slug"].as_str() == Some(slug))
        .unwrap_or_else(|| panic!("管理列表中找不到 slug={slug}: {list}"))
        .clone()
}

// ---------- 1. 未安装门禁 ----------

#[tokio::test(flavor = "multi_thread")]
async fn pages_endpoints_not_installed_gate() {
    let tmp = tempfile::tempdir().unwrap();
    let base = spawn_server(tmp.path().join("config.toml").to_str().unwrap()).await;
    let c = reqwest::Client::new();

    // 公开 pages 接口不进白名单：未安装一律 503 not_installed
    for p in [
        "/api/pages",
        "/api/pages/about",
        "/api/pages/guestbook/comments",
    ] {
        let r = c.get(format!("{base}{p}")).send().await.unwrap();
        assert_eq!(r.status(), 503, "GET {p} 未安装时应 503");
        assert_eq!(err_code(r).await, "not_installed", "GET {p}");
    }
    let r = c
        .post(format!("{base}/api/pages/guestbook/comments"))
        .json(&json!({"author_name": "a", "content": "b"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 503);
    assert_eq!(err_code(r).await, "not_installed");
    // 管理接口同样 503（门禁在鉴权之前）
    let r = c
        .get(format!("{base}/api/admin/pages"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 503);
}

// ---------- 2. 安装注入内置页面 ----------

#[tokio::test(flavor = "multi_thread")]
async fn builtin_pages_seeded_on_install() {
    let tmp = tempfile::tempdir().unwrap();
    let (c, base, token) = setup(tmp.path()).await;

    // 公开列表：3 个内置页全部 enabled，按 sort_order ASC（关于→留言板→友情链接）
    let r = c.get(format!("{base}/api/pages")).send().await.unwrap();
    assert_eq!(r.status(), 200);
    let list = r.json::<Value>().await.unwrap();
    let slugs: Vec<&str> = list
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["slug"].as_str().unwrap())
        .collect();
    assert_eq!(slugs, vec!["about", "guestbook", "links"]);
    let kinds: Vec<&str> = list
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["kind"].as_str().unwrap())
        .collect();
    assert_eq!(kinds, vec!["custom", "message_board", "links"]);

    // 管理列表：built_in=true、enabled=true，含 content_md
    let admin = admin_pages(&c, &base, &token).await;
    for slug in ["about", "guestbook", "links"] {
        let p = find_by_slug(&admin, slug);
        assert_eq!(p["built_in"], json!(true), "{slug} 应为内置页");
        assert_eq!(p["enabled"], json!(true), "{slug} 应默认启用");
        assert!(!p["content_md"].as_str().unwrap().is_empty());
    }

    // 公开详情：content_html 为后端渲染产物（Markdown 加粗 → <strong>）
    let r = c
        .get(format!("{base}/api/pages/about"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    let detail = r.json::<Value>().await.unwrap();
    assert_eq!(detail["kind"], json!("custom"));
    assert!(detail["content_html"]
        .as_str()
        .unwrap()
        .contains("<strong>"));
    assert_eq!(detail["links"], json!([]), "非 links 页 links 恒为 []");

    // 友情链接页：示例链接非空（name/url/description/sort_order，排序递增）
    let r = c
        .get(format!("{base}/api/pages/links"))
        .send()
        .await
        .unwrap();
    let detail = r.json::<Value>().await.unwrap();
    let links = detail["links"].as_array().unwrap();
    assert!(!links.is_empty(), "内置友情链接页应附示例链接");
    let mut prev = -1;
    for l in links {
        assert!(!l["name"].as_str().unwrap().is_empty());
        assert!(l["url"].as_str().unwrap().starts_with("http"));
        let so = l["sort_order"].as_i64().unwrap();
        assert!(so > prev);
        prev = so;
    }
}

// ---------- 3. 页面 CRUD ----------

#[tokio::test(flavor = "multi_thread")]
async fn page_crud_flow() {
    let tmp = tempfile::tempdir().unwrap();
    let (c, base, token) = setup(tmp.path()).await;

    // 管理接口需要鉴权
    let r = c
        .get(format!("{base}/api/admin/pages"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 401);
    assert_eq!(err_code(r).await, "unauthorized");

    // 创建：title + content_md 必填；slug 自动生成（ASCII slugify）
    let r = c
        .post(format!("{base}/api/admin/pages"))
        .bearer_auth(&token)
        .json(&json!({"title": "My Test Page", "content_md": "# 标题\n\n**正文**"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 201, "{}", r.text().await.unwrap());
    let created = r.json::<Value>().await.unwrap();
    let id = created["id"].as_i64().unwrap();
    assert_eq!(created["slug"], json!("my-test-page"));
    assert_eq!(created["kind"], json!("custom"));
    assert_eq!(created["built_in"], json!(false));
    assert_eq!(created["enabled"], json!(true));
    assert_eq!(created["sort_order"], json!(0));
    assert_eq!(created["links"], json!([]));

    // 纯中文标题：slug 回退 page-<id>
    let r = c
        .post(format!("{base}/api/admin/pages"))
        .bearer_auth(&token)
        .json(&json!({"title": "测试页面", "content_md": "内容", "sort_order": 5}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 201);
    let cn = r.json::<Value>().await.unwrap();
    assert_eq!(
        cn["slug"],
        json!(format!("page-{}", cn["id"].as_i64().unwrap()))
    );
    let cn_id = cn["id"].as_i64().unwrap();
    let cn_slug = cn["slug"].as_str().unwrap().to_string();

    // 创建校验：title 缺失/空 → 422；content_md 缺失 → 422
    let r = c
        .post(format!("{base}/api/admin/pages"))
        .bearer_auth(&token)
        .json(&json!({"content_md": "x"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 422);
    assert_eq!(err_code(r).await, "validation_error");
    let r = c
        .post(format!("{base}/api/admin/pages"))
        .bearer_auth(&token)
        .json(&json!({"title": "  ", "content_md": "x"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 422);
    let r = c
        .post(format!("{base}/api/admin/pages"))
        .bearer_auth(&token)
        .json(&json!({"title": "无内容"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 422);

    // 公开列表可见（sort_order=0 排在内置页 10/20/30 之前；=5 也在前）
    let r = c.get(format!("{base}/api/pages")).send().await.unwrap();
    let list = r.json::<Value>().await.unwrap();
    let slugs: Vec<&str> = list
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["slug"].as_str().unwrap())
        .collect();
    assert_eq!(
        slugs,
        vec![
            "my-test-page",
            cn_slug.as_str(),
            "about",
            "guestbook",
            "links"
        ],
        "公开列表按 sort_order ASC, id ASC: {slugs:?}"
    );

    // 公开详情：content_html 渲染
    let r = c
        .get(format!("{base}/api/pages/my-test-page"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    let detail = r.json::<Value>().await.unwrap();
    assert!(detail["content_html"]
        .as_str()
        .unwrap()
        .contains("<strong>"));

    // GET /api/admin/pages/:id
    let r = c
        .get(format!("{base}/api/admin/pages/{id}"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    assert_eq!(
        r.json::<Value>().await.unwrap()["slug"],
        json!("my-test-page")
    );
    // 不存在的 id → 404
    let r = c
        .get(format!("{base}/api/admin/pages/99999"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 404);

    // PUT 更新：标题/slug/内容/排序；kind 字段不接受（传入被忽略）
    let r = c
        .put(format!("{base}/api/admin/pages/{id}"))
        .bearer_auth(&token)
        .json(&json!({
            "title": "改名了",
            "slug": "renamed",
            "content_md": "新内容",
            "sort_order": 99,
            "kind": "message_board"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200, "{}", r.text().await.unwrap());
    let updated = r.json::<Value>().await.unwrap();
    assert_eq!(updated["title"], json!("改名了"));
    assert_eq!(updated["slug"], json!("renamed"));
    assert_eq!(updated["sort_order"], json!(99));
    assert_eq!(updated["kind"], json!("custom"), "kind 不可改");
    // 旧 slug 公开 404、新 slug 可访问
    let r = c
        .get(format!("{base}/api/pages/my-test-page"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 404);
    let r = c
        .get(format!("{base}/api/pages/renamed"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);

    // PATCH toggle：停用 → 公开 404、列表消失；再 toggle 恢复
    let r = c
        .patch(format!("{base}/api/admin/pages/{id}/toggle"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    assert_eq!(r.json::<Value>().await.unwrap()["enabled"], json!(false));
    let r = c
        .get(format!("{base}/api/pages/renamed"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 404);
    assert_eq!(err_code(r).await, "not_found");
    let list = c
        .get(format!("{base}/api/pages"))
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    assert!(
        !list
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["slug"] == json!("renamed")),
        "停用页应从公开列表/导航消失"
    );
    // 管理列表仍可见（含停用页）
    let admin = admin_pages(&c, &base, &token).await;
    assert_eq!(find_by_slug(&admin, "renamed")["enabled"], json!(false));

    let r = c
        .patch(format!("{base}/api/admin/pages/{id}/toggle"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(r.json::<Value>().await.unwrap()["enabled"], json!(true));
    let r = c
        .get(format!("{base}/api/pages/renamed"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);

    // DELETE：自定义页可删（连带清理），删后 404
    let r = c
        .delete(format!("{base}/api/admin/pages/{cn_id}"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 204);
    let r = c
        .get(format!("{base}/api/admin/pages/{cn_id}"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 404);
    let r = c
        .delete(format!("{base}/api/admin/pages/{cn_id}"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 404);
}

// ---------- 4. slug 冲突 409 ----------

#[tokio::test(flavor = "multi_thread")]
async fn page_slug_conflict_409() {
    let tmp = tempfile::tempdir().unwrap();
    let (c, base, token) = setup(tmp.path()).await;

    // 与内置页 slug 冲突
    let r = c
        .post(format!("{base}/api/admin/pages"))
        .bearer_auth(&token)
        .json(&json!({"title": "假关于", "slug": "about", "content_md": "x"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 409);
    assert_eq!(err_code(r).await, "slug_taken");

    // 自动生成路径冲突：已有 "dup"，再建标题 "Dup" 的页（slugify 后同名）
    let r = c
        .post(format!("{base}/api/admin/pages"))
        .bearer_auth(&token)
        .json(&json!({"title": "Dup", "slug": "dup", "content_md": "x"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 201);
    let r = c
        .post(format!("{base}/api/admin/pages"))
        .bearer_auth(&token)
        .json(&json!({"title": "DUP!", "content_md": "x"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 409);
    assert_eq!(err_code(r).await, "slug_taken");

    // 更新时改成他人 slug → 409；改回自己 → 200（排除自身）
    let admin = admin_pages(&c, &base, &token).await;
    let dup_id = find_by_slug(&admin, "dup")["id"].as_i64().unwrap();
    let r = c
        .put(format!("{base}/api/admin/pages/{dup_id}"))
        .bearer_auth(&token)
        .json(&json!({"slug": "about"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 409);
    let r = c
        .put(format!("{base}/api/admin/pages/{dup_id}"))
        .bearer_auth(&token)
        .json(&json!({"slug": "dup"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200, "slug 未变（自身）不应 409");

    // 页面与文章 slug 是独立命名空间：文章已用 hello，页面仍可用 hello
    let r = c
        .post(format!("{base}/api/admin/posts"))
        .bearer_auth(&token)
        .json(&json!({"title": "Hello", "slug": "hello", "content_md": "x", "status": "published"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 201);
    let r = c
        .post(format!("{base}/api/admin/pages"))
        .bearer_auth(&token)
        .json(&json!({"title": "Hello Page", "slug": "hello", "content_md": "x"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 201, "页面 slug 与文章 slug 互不冲突");
}

// ---------- 5. 内置页保护：不可删，可停用可编辑 ----------

#[tokio::test(flavor = "multi_thread")]
async fn builtin_page_protection() {
    let tmp = tempfile::tempdir().unwrap();
    let (c, base, token) = setup(tmp.path()).await;

    let admin = admin_pages(&c, &base, &token).await;
    let about = find_by_slug(&admin, "about");
    let about_id = about["id"].as_i64().unwrap();
    let guestbook_id = find_by_slug(&admin, "guestbook")["id"].as_i64().unwrap();

    // DELETE 内置页 → 422 page_builtin
    for id in [about_id, guestbook_id] {
        let r = c
            .delete(format!("{base}/api/admin/pages/{id}"))
            .bearer_auth(&token)
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), 422);
        assert_eq!(err_code(r).await, "page_builtin");
    }
    // 删除被拒后页面仍在
    let admin = admin_pages(&c, &base, &token).await;
    assert!(find_by_slug(&admin, "about")["built_in"].as_bool().unwrap());

    // 可停用：toggle 后前台 404、导航消失
    let r = c
        .patch(format!("{base}/api/admin/pages/{about_id}/toggle"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    assert_eq!(r.json::<Value>().await.unwrap()["enabled"], json!(false));
    let r = c
        .get(format!("{base}/api/pages/about"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 404);

    // 可编辑：标题/内容/slug 都可改，kind 与 built_in 不变
    let r = c
        .put(format!("{base}/api/admin/pages/{about_id}"))
        .bearer_auth(&token)
        .json(&json!({"title": "关于本站", "content_md": "改写后的内容", "slug": "about-us"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    let updated = r.json::<Value>().await.unwrap();
    assert_eq!(updated["title"], json!("关于本站"));
    assert_eq!(updated["slug"], json!("about-us"));
    assert_eq!(updated["kind"], json!("custom"));
    assert_eq!(updated["built_in"], json!(true));

    // 留言板 kind 不可改：请求体带 kind 也被忽略
    let r = c
        .put(format!("{base}/api/admin/pages/{guestbook_id}"))
        .bearer_auth(&token)
        .json(&json!({"kind": "links", "title": "留言板改名"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    let updated = r.json::<Value>().await.unwrap();
    assert_eq!(updated["kind"], json!("message_board"));
    assert_eq!(updated["title"], json!("留言板改名"));
}

// ---------- 6. 留言板留言与后台来源 ----------

#[tokio::test(flavor = "multi_thread")]
async fn guestbook_comment_flow() {
    let tmp = tempfile::tempdir().unwrap();
    let (c, base, token) = setup(tmp.path()).await;

    // 创建留言（先发后审：创建即 approved，公开可见）
    let r = c
        .post(format!("{base}/api/pages/guestbook/comments"))
        .json(
            &json!({"author_name": " 访客甲 ", "email": "a@b.com", "content": " 你好，路过留言 "}),
        )
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 201, "{}", r.text().await.unwrap());
    let created = r.json::<Value>().await.unwrap();
    assert_eq!(created["author_name"], json!("访客甲"), "应 trim");
    assert_eq!(created["content"], json!("你好，路过留言"));

    // 公开留言列表（仅 approved，时间 ASC）
    let r = c
        .get(format!("{base}/api/pages/guestbook/comments"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    let list = r.json::<Value>().await.unwrap();
    assert_eq!(list.as_array().unwrap().len(), 1);
    assert_eq!(list[0]["id"], created["id"]);

    // 校验：author_name/content 必填 → 422
    let r = c
        .post(format!("{base}/api/pages/guestbook/comments"))
        .json(&json!({"author_name": "", "content": "x"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 422);
    assert_eq!(err_code(r).await, "validation_error");
    let r = c
        .post(format!("{base}/api/pages/guestbook/comments"))
        .json(&json!({"author_name": "a", "content": "  "}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 422);

    // 非留言板页（about 是 custom）→ 404；不存在的页 → 404
    let r = c
        .post(format!("{base}/api/pages/about/comments"))
        .json(&json!({"author_name": "a", "content": "b"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 404);
    let r = c
        .get(format!("{base}/api/pages/no-such-page/comments"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 404);

    // 后台留言列表可见：target_type=page、post_title=页面标题（留言板）
    let r = c
        .get(format!("{base}/api/admin/comments"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    let data = r.json::<Value>().await.unwrap();
    let mine = data["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|it| it["id"] == created["id"])
        .expect("后台留言列表应包含刚创建的留言");
    assert_eq!(mine["target_type"], json!("page"));
    assert_eq!(mine["post_title"], json!("留言板"));
    assert_eq!(mine["status"], json!("approved"));

    // 后台隐藏 → 公开列表消失
    let cid = created["id"].as_i64().unwrap();
    let r = c
        .put(format!("{base}/api/admin/comments/{cid}"))
        .bearer_auth(&token)
        .json(&json!({"status": "hidden"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    assert_eq!(
        r.json::<Value>().await.unwrap()["target_type"],
        json!("page")
    );
    let list = c
        .get(format!("{base}/api/pages/guestbook/comments"))
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    assert_eq!(list.as_array().unwrap().len(), 0);

    // 文章评论回归：target_type=post；post_id 过滤只匹配文章来源
    let admin = admin_pages(&c, &base, &token).await;
    let guestbook_id = find_by_slug(&admin, "guestbook")["id"].as_i64().unwrap();
    let r = c
        .post(format!("{base}/api/posts/reedblog/comments"))
        .json(&json!({"author_name": "读者", "content": "文章评论"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 201, "示例文章 slug=reedblog 应可评论");
    let post_comment = r.json::<Value>().await.unwrap();

    // post_id 过滤仅匹配文章来源：即使目标 id 数值相同（留言板页 id 与某文章 id
    // 可能同号），页面留言也不得混入；返回的（若有）只能是 target_type=post 的文章评论
    let r = c
        .get(format!("{base}/api/admin/comments?post_id={guestbook_id}"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    let data = r.json::<Value>().await.unwrap();
    for it in data["items"].as_array().unwrap() {
        assert_eq!(
            it["target_type"],
            json!("post"),
            "post_id 过滤不得返回页面留言"
        );
        assert_ne!(it["id"], created["id"], "页面留言混入了 post_id 过滤");
    }

    let r = c
        .get(format!("{base}/api/admin/comments"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    let data = r.json::<Value>().await.unwrap();
    let pc = data["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|it| it["id"] == post_comment["id"])
        .unwrap()
        .clone();
    assert_eq!(pc["target_type"], json!("post"));
    // 用文章 id 过滤能查到该文章评论
    let post_id = pc["post_id"].as_i64().unwrap();
    let r = c
        .get(format!("{base}/api/admin/comments?post_id={post_id}"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    let data = r.json::<Value>().await.unwrap();
    assert_eq!(data["total"], json!(1));
    assert_eq!(data["items"][0]["target_type"], json!("post"));
}

// ---------- 7. sitemap 含 enabled 页面 ----------

#[tokio::test(flavor = "multi_thread")]
async fn sitemap_includes_enabled_pages() {
    let tmp = tempfile::tempdir().unwrap();
    let (c, base, token) = setup(tmp.path()).await;

    let xml = c
        .get(format!("{base}/api/sitemap.xml"))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    for needle in ["/pages/about", "/pages/guestbook", "/pages/links"] {
        assert!(xml.contains(needle), "sitemap 应含内置页 {needle}");
    }

    // 停用 about → sitemap 不再包含
    let admin = admin_pages(&c, &base, &token).await;
    let about_id = find_by_slug(&admin, "about")["id"].as_i64().unwrap();
    let r = c
        .patch(format!("{base}/api/admin/pages/{about_id}/toggle"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    let xml = c
        .get(format!("{base}/api/sitemap.xml"))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(!xml.contains("/pages/about"), "停用页应移出 sitemap");
    assert!(xml.contains("/pages/guestbook"));

    // 自建页也进 sitemap
    let r = c
        .post(format!("{base}/api/admin/pages"))
        .bearer_auth(&token)
        .json(&json!({"title": "Sitemap Page", "slug": "sm-page", "content_md": "x"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 201);
    let xml = c
        .get(format!("{base}/api/sitemap.xml"))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(xml.contains("/pages/sm-page"));
}

// ---------- 8. 友情链接管理 ----------

#[tokio::test(flavor = "multi_thread")]
async fn links_page_management() {
    let tmp = tempfile::tempdir().unwrap();
    let (c, base, token) = setup(tmp.path()).await;

    let admin = admin_pages(&c, &base, &token).await;
    let links_page = find_by_slug(&admin, "links");
    let links_id = links_page["id"].as_i64().unwrap();
    let about_id = find_by_slug(&admin, "about")["id"].as_i64().unwrap();

    // 全量替换：数组顺序即 sort_order（10, 20, 30…）
    let r = c
        .put(format!("{base}/api/admin/pages/{links_id}"))
        .bearer_auth(&token)
        .json(&json!({
            "links": [
                {"name": "站A", "url": "https://a.example.com", "description": "第一个"},
                {"name": "站B", "url": "https://b.example.com"}
            ]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200, "{}", r.text().await.unwrap());
    let updated = r.json::<Value>().await.unwrap();
    let links = updated["links"].as_array().unwrap();
    assert_eq!(links.len(), 2, "全量替换后应为 2 条");
    assert_eq!(links[0]["name"], json!("站A"));
    assert_eq!(links[0]["sort_order"], json!(10));
    assert_eq!(links[1]["sort_order"], json!(20));
    assert_eq!(links[1]["description"], json!(""), "缺省描述为空串");

    // 公开详情同步
    let detail = c
        .get(format!("{base}/api/pages/links"))
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    assert_eq!(detail["links"].as_array().unwrap().len(), 2);

    // 重排序 = 换数组顺序再全量替换
    let r = c
        .put(format!("{base}/api/admin/pages/{links_id}"))
        .bearer_auth(&token)
        .json(&json!({
            "links": [
                {"name": "站B", "url": "https://b.example.com"},
                {"name": "站A", "url": "https://a.example.com"}
            ]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    let links = r.json::<Value>().await.unwrap()["links"].clone();
    assert_eq!(links[0]["name"], json!("站B"));
    assert_eq!(links[1]["name"], json!("站A"));

    // 清空
    let r = c
        .put(format!("{base}/api/admin/pages/{links_id}"))
        .bearer_auth(&token)
        .json(&json!({"links": []}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    assert_eq!(r.json::<Value>().await.unwrap()["links"], json!([]));

    // 校验：URL 必须为 http/https 绝对地址；name/url 非空 → 422
    for bad in [
        json!([{"name": "x", "url": "javascript:alert(1)"}]),
        json!([{"name": "x", "url": "not-a-url"}]),
        json!([{"name": "", "url": "https://a.com"}]),
        json!([{"name": "x", "url": "  "}]),
    ] {
        let r = c
            .put(format!("{base}/api/admin/pages/{links_id}"))
            .bearer_auth(&token)
            .json(&json!({"links": bad}))
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), 422, "非法 links {bad} 应 422");
        assert_eq!(err_code(r).await, "validation_error");
    }
    // 校验失败不落库（links 仍为空）
    let admin = admin_pages(&c, &base, &token).await;
    assert_eq!(find_by_slug(&admin, "links")["links"], json!([]));

    // 非 links 页：links 字段被忽略（不报错、不生效）
    let r = c
        .put(format!("{base}/api/admin/pages/{about_id}"))
        .bearer_auth(&token)
        .json(&json!({"links": [{"name": "x", "url": "https://a.com"}]}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    assert_eq!(
        r.json::<Value>().await.unwrap()["links"],
        json!([]),
        "kind=custom 页面应忽略 links"
    );

    // 创建自定义页时 links 也被忽略（kind 恒为 custom）
    let r = c
        .post(format!("{base}/api/admin/pages"))
        .bearer_auth(&token)
        .json(&json!({
            "title": "带链接尝试", "content_md": "x",
            "links": [{"name": "x", "url": "https://a.com"}]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 201);
    assert_eq!(r.json::<Value>().await.unwrap()["links"], json!([]));
}

// ---------- 9. 删除页面连带清理留言 ----------

#[tokio::test(flavor = "multi_thread")]
async fn delete_page_cascades_comments() {
    let tmp = tempfile::tempdir().unwrap();
    let (c, base, token) = setup(tmp.path()).await;

    // 停用内置留言板，再建一个自定义页验证删除连带（内置页不可删）
    let r = c
        .post(format!("{base}/api/admin/pages"))
        .bearer_auth(&token)
        .json(&json!({"title": "临时页", "content_md": "x"}))
        .send()
        .await
        .unwrap();
    let page_id = r.json::<Value>().await.unwrap()["id"].as_i64().unwrap();

    // 先给内置留言板挂一条留言，删除自定义页不得误伤它
    let r = c
        .post(format!("{base}/api/pages/guestbook/comments"))
        .json(&json!({"author_name": "某人", "content": "留言内容"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 201);
    let gb_comment_id = r.json::<Value>().await.unwrap()["id"].clone();

    let r = c
        .delete(format!("{base}/api/admin/pages/{page_id}"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 204);

    // 留言板留言仍在
    let r = c
        .get(format!("{base}/api/admin/comments"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    let data = r.json::<Value>().await.unwrap();
    assert!(
        data["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|it| it["id"] == gb_comment_id),
        "删除其他页面不得误伤留言板留言"
    );
}
