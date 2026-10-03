//! 文章置顶 + 定时发布集成测试（契约「文章置顶与定时发布」条款）：
//! - 置顶排序：公开列表 recent 序 is_sticky DESC, published_at DESC（标签/分类过滤同规则）；
//!   hot 序、搜索、RSS、sitemap 不受置顶影响
//! - sticky PATCH：鉴权 / 404 / 切换回读；POST/PUT 的 is_sticky 字段
//! - 定时发布（惰性方案）：到点前公开渠道全不可见（列表/详情/搜索/评论目标/标签分类计数/
//!   归档/RSS/sitemap/站点统计），到点后（直接改库把 published_at 拨到过去模拟时间流逝）
//!   **无需重启**即全部可见；后台管理列表全程可见并可按 scheduled 过滤
//! - 校验：scheduled 缺时间/过去时间/非法格式 → 422 validation_error；
//!   published→scheduled → 422；scheduled→published 立即发布；scheduled→draft 清空计划时间；
//!   已到点的 scheduled 文章常规编辑不重新校验未来时间

use reedblog_backend::state::AppState;
use serde_json::{json, Value};
use std::path::Path;

/// 在 127.0.0.1 随机端口起真实服务（与 integration.rs 同款隔离：插件/主题目录进 tempdir）
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

/// 安装 + 登录，返回 Bearer token（安装注入 3 篇示例文章：2026-09-20 / 08-15 / 07-08）
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

/// 安装写入的 config.toml → SQLite 连接 URL（复用 Config::db_url 的路径编码逻辑）
fn sqlite_url_from_config(config_path: &str) -> String {
    reedblog_backend::config::Config::load(Path::new(config_path))
        .expect("config.toml 应已由安装流程写入")
        .db_url()
        .expect("db_type 应为 sqlite")
}

/// 直接改库构造 API 校验写不进去的数据（published_at 在过去的 scheduled 文章，
/// 即「到点」状态——惰性发布方案下 status 仍为 scheduled，靠查询条件放行）
async fn db_set_published_at(url: &str, post_id: i64, ts: &str) {
    sqlx::any::install_default_drivers();
    let pool = sqlx::AnyPool::connect(url).await.unwrap();
    sqlx::query("UPDATE posts SET published_at = ? WHERE id = ?")
        .bind(ts)
        .bind(post_id)
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;
}

fn rfc3339_secs(dt: chrono::DateTime<chrono::Utc>) -> String {
    dt.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

/// GET 公开接口并解析 JSON
async fn get_json(c: &reqwest::Client, url: &str) -> Value {
    c.get(url)
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap()
}

/// 带 Bearer 的管理请求并解析 JSON
async fn admin_json(
    c: &reqwest::Client,
    method: &str,
    url: &str,
    token: &str,
    body: Option<Value>,
) -> (u16, Value) {
    let mut req = c.request(method.parse().unwrap(), url).bearer_auth(token);
    if let Some(b) = body {
        req = req.json(&b);
    }
    let r = req.send().await.unwrap();
    let status = r.status().as_u16();
    (status, r.json::<Value>().await.unwrap())
}

/// 公开文章列表的 slug 序列
async fn public_slugs(c: &reqwest::Client, base: &str) -> Vec<String> {
    let v = get_json(c, &format!("{base}/api/posts")).await;
    v["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["slug"].as_str().unwrap().to_string())
        .collect()
}

/// feed.xml 中 item 的 link 顺序（第一个 <link> 是 channel 的，跳过）
fn feed_item_links(xml: &str) -> Vec<String> {
    xml.split("<link>")
        .skip(1)
        .map(|s| s.split("</link>").next().unwrap().to_string())
        .skip(1)
        .collect()
}

/// sitemap.xml 中 <loc> 顺序
fn sitemap_locs(xml: &str) -> Vec<String> {
    xml.split("<loc>")
        .skip(1)
        .map(|s| s.split("</loc>").next().unwrap().to_string())
        .collect()
}

// ---------- 1. 置顶：排序（列表/标签/分类）+ PATCH + 形状 ----------

#[tokio::test(flavor = "multi_thread")]
async fn sticky_ordering_and_patch() {
    let tmp = tempfile::tempdir().unwrap();
    let base = spawn_server(tmp.path().join("config.toml").to_str().unwrap()).await;
    let c = reqwest::Client::new();
    let token = setup_installed(&c, &base, tmp.path()).await;

    // 初始公开列表：3 篇示例按 published_at DESC，is_sticky 均为 false（形状新增字段）
    let v = get_json(&c, &format!("{base}/api/posts")).await;
    assert_eq!(v["total"], 3);
    let items = v["items"].as_array().unwrap();
    assert_eq!(items[0]["title"], "欢迎使用 reedblog");
    assert_eq!(items[2]["title"], "周末随笔：慢下来的时光");
    for it in items {
        assert_eq!(it["is_sticky"], false, "旧数据默认不置顶: {it}");
    }
    let oldest_id = items[2]["id"].as_i64().unwrap();
    let oldest_slug = items[2]["slug"].as_str().unwrap().to_string();

    // PATCH sticky：未带 token → 401；id 不存在 → 404；缺 is_sticky → 422
    let r = c
        .patch(format!("{base}/api/admin/posts/{oldest_id}/sticky"))
        .json(&json!({"is_sticky": true}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 401);
    assert_eq!(err_code(r).await, "unauthorized");
    let r = c
        .patch(format!("{base}/api/admin/posts/99999/sticky"))
        .bearer_auth(&token)
        .json(&json!({"is_sticky": true}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 404);
    assert_eq!(err_code(r).await, "not_found");
    let r = c
        .patch(format!("{base}/api/admin/posts/{oldest_id}/sticky"))
        .bearer_auth(&token)
        .json(&json!({}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 422);
    assert_eq!(err_code(r).await, "validation_error");

    // 置顶最老一篇 → PostAdmin 回读 is_sticky=true、status 不变
    let (st, v) = admin_json(
        &c,
        "PATCH",
        &format!("{base}/api/admin/posts/{oldest_id}/sticky"),
        &token,
        Some(json!({"is_sticky": true})),
    )
    .await;
    assert_eq!(st, 200);
    assert_eq!(v["is_sticky"], true);
    assert_eq!(v["status"], "published");
    assert_eq!(v["id"], oldest_id);

    // 公开列表：置顶排最前（is_sticky DESC, published_at DESC），徽章字段随行
    let v = get_json(&c, &format!("{base}/api/posts")).await;
    let items = v["items"].as_array().unwrap();
    assert_eq!(items[0]["slug"], oldest_slug, "置顶文章应排最前");
    assert_eq!(items[0]["is_sticky"], true);
    assert_eq!(items[1]["title"], "欢迎使用 reedblog");
    assert_eq!(items[1]["is_sticky"], false);
    // 详情同形状
    let v = get_json(&c, &format!("{base}/api/posts/{oldest_slug}")).await;
    assert_eq!(v["is_sticky"], true);

    // 取消置顶 → 回到纯时间序
    let (st, v) = admin_json(
        &c,
        "PATCH",
        &format!("{base}/api/admin/posts/{oldest_id}/sticky"),
        &token,
        Some(json!({"is_sticky": false})),
    )
    .await;
    assert_eq!(st, 200);
    assert_eq!(v["is_sticky"], false);
    let v = get_json(&c, &format!("{base}/api/posts")).await;
    assert_eq!(v["items"][0]["title"], "欢迎使用 reedblog");

    // 标签列表同规则：tag=随笔 命中 [欢迎(09-20), 周末(07-08)]，置顶周末后排最前
    let v = get_json(&c, &format!("{base}/api/posts?tag=随笔")).await;
    assert_eq!(v["total"], 2);
    assert_eq!(v["items"][0]["title"], "欢迎使用 reedblog");
    let (_, _) = admin_json(
        &c,
        "PATCH",
        &format!("{base}/api/admin/posts/{oldest_id}/sticky"),
        &token,
        Some(json!({"is_sticky": true})),
    )
    .await;
    let v = get_json(&c, &format!("{base}/api/posts?tag=随笔")).await;
    assert_eq!(
        v["items"][0]["title"], "周末随笔：慢下来的时光",
        "标签列表置顶也应排最前"
    );
    assert_eq!(v["items"][0]["is_sticky"], true);
    let (_, _) = admin_json(
        &c,
        "PATCH",
        &format!("{base}/api/admin/posts/{oldest_id}/sticky"),
        &token,
        Some(json!({"is_sticky": false})),
    )
    .await;

    // 分类列表同规则：同分类两篇 A/B（同秒创建，时间序不定），置顶 A 后 A 必在最前
    let (st, cat) = admin_json(
        &c,
        "POST",
        &format!("{base}/api/admin/categories"),
        &token,
        Some(json!({"name": "置顶测试"})),
    )
    .await;
    assert_eq!(st, 201);
    let cat_id = cat["id"].as_i64().unwrap();
    for (title, slug) in [("Sticky A", "sticky-a"), ("Sticky B", "sticky-b")] {
        let (st, _) = admin_json(
            &c,
            "POST",
            &format!("{base}/api/admin/posts"),
            &token,
            Some(json!({
                "title": title, "slug": slug, "content_md": "x",
                "status": "published", "category_id": cat_id
            })),
        )
        .await;
        assert_eq!(st, 201);
    }
    let v = get_json(&c, &format!("{base}/api/posts?category=置顶测试")).await;
    assert_eq!(v["total"], 2);
    let a_id = v["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["slug"] == "sticky-a")
        .unwrap()["id"]
        .as_i64()
        .unwrap();
    let (st, _) = admin_json(
        &c,
        "PATCH",
        &format!("{base}/api/admin/posts/{a_id}/sticky"),
        &token,
        Some(json!({"is_sticky": true})),
    )
    .await;
    assert_eq!(st, 200);
    let v = get_json(&c, &format!("{base}/api/posts?category=置顶测试")).await;
    let items = v["items"].as_array().unwrap();
    assert_eq!(items[0]["slug"], "sticky-a", "分类列表置顶应排最前");
    assert_eq!(items[0]["is_sticky"], true);
    assert_eq!(items[1]["slug"], "sticky-b");
    assert_eq!(items[1]["is_sticky"], false);

    // POST/PUT 也接受 is_sticky（编辑器置顶开关）：创建即置顶 → 更新取消 → 缺省保持
    let (st, v) = admin_json(
        &c,
        "POST",
        &format!("{base}/api/admin/posts"),
        &token,
        Some(json!({
            "title": "Sticky C", "slug": "sticky-c", "content_md": "x",
            "status": "draft", "is_sticky": true
        })),
    )
    .await;
    assert_eq!(st, 201);
    let c_id = v["id"].as_i64().unwrap();
    assert_eq!(v["is_sticky"], true);
    let (_, v) = admin_json(
        &c,
        "PUT",
        &format!("{base}/api/admin/posts/{c_id}"),
        &token,
        Some(json!({"is_sticky": false})),
    )
    .await;
    assert_eq!(v["is_sticky"], false);
    let (_, v) = admin_json(
        &c,
        "PUT",
        &format!("{base}/api/admin/posts/{c_id}"),
        &token,
        Some(json!({"excerpt": "只改摘要"})),
    )
    .await;
    assert_eq!(v["is_sticky"], false, "PUT 缺省 is_sticky 应保持原值");
    assert_eq!(v["excerpt"], "只改摘要");
    // 默认（不提供 is_sticky）创建 → false
    let (_, v) = admin_json(
        &c,
        "POST",
        &format!("{base}/api/admin/posts"),
        &token,
        Some(json!({"title": "Plain", "slug": "plain", "content_md": "x", "status": "draft"})),
    )
    .await;
    assert_eq!(v["is_sticky"], false);

    // 管理列表回读 is_sticky（sticky-a 仍置顶）
    let v = get_json_authed(&c, &format!("{base}/api/admin/posts?status=all"), &token).await;
    let a = v["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["slug"] == "sticky-a")
        .unwrap();
    assert_eq!(a["is_sticky"], true);
}

/// 带 Bearer 的 GET 并解析 JSON
async fn get_json_authed(c: &reqwest::Client, url: &str, token: &str) -> Value {
    c.get(url)
        .bearer_auth(token)
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap()
}

// ---------- 2. RSS / sitemap / hot 序不受置顶影响 ----------

#[tokio::test(flavor = "multi_thread")]
async fn rss_sitemap_hot_unaffected_by_sticky() {
    let tmp = tempfile::tempdir().unwrap();
    let base = spawn_server(tmp.path().join("config.toml").to_str().unwrap()).await;
    let c = reqwest::Client::new();
    let token = setup_installed(&c, &base, tmp.path()).await;

    let feed_before = c
        .get(format!("{base}/api/feed.xml"))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    let links_before = feed_item_links(&feed_before);
    assert_eq!(links_before.len(), 3, "feed 应含 3 篇示例文章");
    let sitemap_before = sitemap_locs(
        &c.get(format!("{base}/api/sitemap.xml"))
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap(),
    );
    let hot_before = get_json(&c, &format!("{base}/api/posts?order=hot")).await;
    assert_eq!(
        hot_before["items"][0]["slug"], "axum-sqlx",
        "hot 序按评论数：axum-sqlx 有 1 条示例评论应最前"
    );

    // 置顶最老一篇（纯时间序下排最后）
    let v = get_json(&c, &format!("{base}/api/posts")).await;
    let items = v["items"].as_array().unwrap();
    let oldest_id = items[2]["id"].as_i64().unwrap();
    let oldest_slug = items[2]["slug"].as_str().unwrap().to_string();
    let (st, _) = admin_json(
        &c,
        "PATCH",
        &format!("{base}/api/admin/posts/{oldest_id}/sticky"),
        &token,
        Some(json!({"is_sticky": true})),
    )
    .await;
    assert_eq!(st, 200);

    // 公开 recent 列表：置顶生效排最前
    assert_eq!(public_slugs(&c, &base).await[0], oldest_slug);

    // RSS：item 顺序保持纯时间序（published_at DESC），完全不变
    let feed_after = c
        .get(format!("{base}/api/feed.xml"))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert_eq!(
        feed_item_links(&feed_after),
        links_before,
        "RSS 不应受置顶影响"
    );

    // sitemap：loc 顺序不变
    let sitemap_after = sitemap_locs(
        &c.get(format!("{base}/api/sitemap.xml"))
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap(),
    );
    assert_eq!(sitemap_after, sitemap_before, "sitemap 不应受置顶影响");

    // hot 序不变（仍按评论数）
    let hot_after = get_json(&c, &format!("{base}/api/posts?order=hot")).await;
    assert_eq!(hot_after["items"][0]["slug"], "axum-sqlx");

    // 搜索序不受置顶影响：按 published_at DESC，置顶的最老一篇仍排最后
    let v = get_json(&c, &format!("{base}/api/search?q=的")).await;
    let slugs: Vec<&str> = v["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["slug"].as_str().unwrap())
        .collect();
    assert_eq!(
        *slugs.last().unwrap(),
        oldest_slug.as_str(),
        "搜索结果应保持时间序，不受置顶影响"
    );
}

// ---------- 3. 定时发布：到点前不可见 → 到点后无需重启即可见 ----------

#[tokio::test(flavor = "multi_thread")]
async fn scheduled_lazy_visibility() {
    let tmp = tempfile::tempdir().unwrap();
    let cfg_path = tmp.path().join("config.toml");
    let base = spawn_server(cfg_path.to_str().unwrap()).await;
    let c = reqwest::Client::new();
    let token = setup_installed(&c, &base, tmp.path()).await;
    let db_url = sqlite_url_from_config(cfg_path.to_str().unwrap());

    // 分类 + 标签（验证计数口径）
    let (_, cat) = admin_json(
        &c,
        "POST",
        &format!("{base}/api/admin/categories"),
        &token,
        Some(json!({"name": "定时分类"})),
    )
    .await;
    let cat_id = cat["id"].as_i64().unwrap();
    let (_, tag) = admin_json(
        &c,
        "POST",
        &format!("{base}/api/admin/tags"),
        &token,
        Some(json!({"name": "定时标签"})),
    )
    .await;
    let tag_id = tag["id"].as_i64().unwrap();

    // 创建 scheduled 文章：计划时间 = 1 小时后（未来）
    let future = rfc3339_secs(chrono::Utc::now() + chrono::Duration::hours(1));
    let (st, sched) = admin_json(
        &c,
        "POST",
        &format!("{base}/api/admin/posts"),
        &token,
        Some(json!({
            "title": "定时发布测试", "slug": "sched-post",
            "content_md": "这是定时发布的独特正文标记",
            "status": "scheduled", "published_at": &future,
            "category_id": cat_id, "tag_ids": [tag_id]
        })),
    )
    .await;
    assert_eq!(st, 201);
    let sched_id = sched["id"].as_i64().unwrap();
    assert_eq!(sched["status"], "scheduled");
    assert_eq!(sched["published_at"], future);
    assert_eq!(sched["is_sticky"], false);

    // ---- 到点前：公开渠道全部不可见 ----
    let v = get_json(&c, &format!("{base}/api/posts")).await;
    assert_eq!(v["total"], 3, "未到点的 scheduled 不进公开列表");
    assert!(!public_slugs(&c, &base)
        .await
        .contains(&"sched-post".to_string()));
    let r = c
        .get(format!("{base}/api/posts/sched-post"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 404, "未到点详情应 404");
    assert_eq!(err_code(r).await, "not_found");
    // 评论目标可见性：GET/POST 均 404
    let r = c
        .get(format!("{base}/api/posts/sched-post/comments"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 404);
    let r = c
        .post(format!("{base}/api/posts/sched-post/comments"))
        .json(&json!({"author_name": "读者", "content": "沙发"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 404, "未到点的文章不能收评论");
    // 搜索不可见
    let v = get_json(&c, &format!("{base}/api/search?q=定时发布")).await;
    assert_eq!(v["total"], 0);
    // 计数口径：分类/标签 post_count 不含它
    let v = get_json(&c, &format!("{base}/api/categories")).await;
    let cat_row = v
        .as_array()
        .unwrap()
        .iter()
        .find(|x| x["name"] == "定时分类")
        .unwrap()
        .clone();
    assert_eq!(cat_row["post_count"], 0);
    let v = get_json(&c, &format!("{base}/api/tags")).await;
    let tag_row = v
        .as_array()
        .unwrap()
        .iter()
        .find(|x| x["name"] == "定时标签")
        .unwrap()
        .clone();
    assert_eq!(tag_row["post_count"], 0);
    // 站点统计
    let v = get_json(&c, &format!("{base}/api/site/stats")).await;
    assert_eq!(v["post_count"], 3);
    // 归档：计数总和仍为 3（计划时间所在月不计入）
    let v = get_json(&c, &format!("{base}/api/archive")).await;
    let archive_sum: i64 = v
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["count"].as_i64().unwrap())
        .sum();
    assert_eq!(archive_sum, 3, "未到点不进归档计数");
    // RSS / sitemap 不含
    let feed = c
        .get(format!("{base}/api/feed.xml"))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(!feed.contains("sched-post"));
    let sitemap = c
        .get(format!("{base}/api/sitemap.xml"))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(!sitemap.contains("sched-post"));

    // ---- 后台全程可见 + status 过滤 ----
    let v = get_json_authed(
        &c,
        &format!("{base}/api/admin/posts?status=scheduled"),
        &token,
    )
    .await;
    assert_eq!(v["total"], 1);
    assert_eq!(v["items"][0]["id"], sched_id);
    assert_eq!(v["items"][0]["status"], "scheduled");
    assert_eq!(v["items"][0]["published_at"], future, "后台显示计划时间");
    let v = get_json_authed(
        &c,
        &format!("{base}/api/admin/posts?status=published"),
        &token,
    )
    .await;
    assert_eq!(v["total"], 3, "scheduled 不算 published");
    let v = get_json_authed(&c, &format!("{base}/api/admin/posts?status=all"), &token).await;
    assert_eq!(v["total"], 4, "all 含 scheduled");
    let r = c
        .get(format!("{base}/api/admin/posts?status=bogus"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 422);
    assert_eq!(err_code(r).await, "validation_error");

    // ---- 模拟到点：直接把 published_at 拨到 1 分钟前（status 保持 scheduled）----
    let past = rfc3339_secs(chrono::Utc::now() - chrono::Duration::seconds(60));
    db_set_published_at(&db_url, sched_id, &past).await;

    // 同一进程、同一 base URL——不重启即全部可见（惰性发布）
    let r = c
        .get(format!("{base}/api/posts/sched-post"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200, "到点后详情应可见（无需重启）");
    let v: Value = r.json().await.unwrap();
    assert_eq!(v["title"], "定时发布测试");
    assert_eq!(v["is_sticky"], false);
    assert!(public_slugs(&c, &base)
        .await
        .contains(&"sched-post".to_string()));
    let v = get_json(&c, &format!("{base}/api/posts")).await;
    assert_eq!(v["total"], 4);
    // 搜索可见
    let v = get_json(&c, &format!("{base}/api/search?q=定时发布")).await;
    assert_eq!(v["total"], 1);
    assert_eq!(v["items"][0]["slug"], "sched-post");
    // 评论目标可见：GET → []，POST → 201
    let v = c
        .get(format!("{base}/api/posts/sched-post/comments"))
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    assert_eq!(v.as_array().unwrap().len(), 0);
    let r = c
        .post(format!("{base}/api/posts/sched-post/comments"))
        .json(&json!({"author_name": "读者", "content": "到点后的沙发"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 201);
    // 计数联动：分类/标签/站点统计/归档
    let v = get_json(&c, &format!("{base}/api/categories")).await;
    let cat_row = v
        .as_array()
        .unwrap()
        .iter()
        .find(|x| x["name"] == "定时分类")
        .unwrap()
        .clone();
    assert_eq!(cat_row["post_count"], 1, "到点后分类计数应含它");
    let v = get_json(&c, &format!("{base}/api/tags")).await;
    let tag_row = v
        .as_array()
        .unwrap()
        .iter()
        .find(|x| x["name"] == "定时标签")
        .unwrap()
        .clone();
    assert_eq!(tag_row["post_count"], 1);
    let v = get_json(&c, &format!("{base}/api/site/stats")).await;
    assert_eq!(v["post_count"], 4);
    let v = get_json(&c, &format!("{base}/api/archive")).await;
    let arr = v.as_array().unwrap();
    let archive_sum: i64 = arr.iter().map(|e| e["count"].as_i64().unwrap()).sum();
    assert_eq!(archive_sum, 4, "到点后应计入归档");
    let (py, pm) = (
        past[..4].parse::<i64>().unwrap(),
        past[5..7].parse::<i64>().unwrap(),
    );
    assert!(
        arr.iter()
            .any(|e| e["year"] == py && e["month"] == pm && e["count"].as_i64().unwrap() >= 1),
        "归档应含计划时间所在年月: {arr:?}"
    );
    // RSS / sitemap 收录
    let feed = c
        .get(format!("{base}/api/feed.xml"))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(feed.contains("sched-post"), "到点后应进 RSS");
    let sitemap = c
        .get(format!("{base}/api/sitemap.xml"))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(sitemap.contains("sched-post"), "到点后应进 sitemap");

    // 已到点的 scheduled 文章常规编辑（不触碰 status/published_at）不重新校验未来时间
    let (st, v) = admin_json(
        &c,
        "PUT",
        &format!("{base}/api/admin/posts/{sched_id}"),
        &token,
        Some(json!({"title": "定时发布测试改题"})),
    )
    .await;
    assert_eq!(st, 200, "到点后编辑不应被未来时间校验挡住");
    assert_eq!(v["status"], "scheduled");
    assert_eq!(v["published_at"], past, "计划时间保持原值");
    assert_eq!(v["title"], "定时发布测试改题");
    let v = get_json(&c, &format!("{base}/api/posts/sched-post")).await;
    assert_eq!(v["title"], "定时发布测试改题");
}

// ---------- 4. 定时发布：校验与状态转换 ----------

#[tokio::test(flavor = "multi_thread")]
async fn scheduled_validation_and_transitions() {
    let tmp = tempfile::tempdir().unwrap();
    let base = spawn_server(tmp.path().join("config.toml").to_str().unwrap()).await;
    let c = reqwest::Client::new();
    let token = setup_installed(&c, &base, tmp.path()).await;

    let now = chrono::Utc::now();
    let past = rfc3339_secs(now - chrono::Duration::hours(1));
    let future_dt = now + chrono::Duration::hours(3);
    let future = rfc3339_secs(future_dt);

    // POST scheduled 缺 published_at → 422 validation_error
    for (label, body) in [
        (
            "缺时间",
            json!({"title": "S1", "content_md": "x", "status": "scheduled"}),
        ),
        (
            "过去时间",
            json!({"title": "S2", "content_md": "x", "status": "scheduled", "published_at": &past}),
        ),
        (
            "非法格式",
            json!({"title": "S3", "content_md": "x", "status": "scheduled", "published_at": "not-a-date"}),
        ),
        (
            "空串",
            json!({"title": "S4", "content_md": "x", "status": "scheduled", "published_at": "  "}),
        ),
    ] {
        let r = c
            .post(format!("{base}/api/admin/posts"))
            .bearer_auth(&token)
            .json(&body)
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), 422, "{label} 应 422");
        assert_eq!(err_code(r).await, "validation_error", "{label}");
    }

    // POST scheduled 带毫秒/时区偏移的时间 → 归一化为 UTC 秒精度存储
    let (st, sched) = admin_json(
        &c,
        "POST",
        &format!("{base}/api/admin/posts"),
        &token,
        Some(json!({
            "title": "Sched OK", "slug": "sched-ok", "content_md": "x",
            "status": "scheduled", "published_at": future_dt.to_rfc3339()
        })),
    )
    .await;
    assert_eq!(st, 201);
    let sched_id = sched["id"].as_i64().unwrap();
    assert_eq!(
        sched["published_at"], future,
        "写入应归一化为 UTC 秒精度（如 2026-10-03T12:00:00Z）"
    );

    // POST body 的 published_at 在非 scheduled 状态下忽略（published 首发布仍写 now）
    let (st, v) = admin_json(
        &c,
        "POST",
        &format!("{base}/api/admin/posts"),
        &token,
        Some(json!({
            "title": "Pub Now", "slug": "pub-now", "content_md": "x",
            "status": "published", "published_at": &future
        })),
    )
    .await;
    assert_eq!(st, 201);
    let pub_id = v["id"].as_i64().unwrap();
    let pa = v["published_at"].as_str().unwrap().to_string();
    assert!(
        pa <= rfc3339_secs(chrono::Utc::now()),
        "published 首发布应写 now"
    );

    // published → scheduled 拒绝 → 422（先转草稿）
    let r = c
        .put(format!("{base}/api/admin/posts/{pub_id}"))
        .bearer_auth(&token)
        .json(&json!({"status": "scheduled", "published_at": &future}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 422);
    assert_eq!(err_code(r).await, "validation_error");
    // 状态未被改动
    let v = get_json_authed(&c, &format!("{base}/api/admin/posts/{pub_id}"), &token).await;
    assert_eq!(v["status"], "published");

    // scheduled 改期：过去时间 → 422；新未来时间 → 200 且 published_at 更新
    let r = c
        .put(format!("{base}/api/admin/posts/{sched_id}"))
        .bearer_auth(&token)
        .json(&json!({"published_at": &past}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 422, "改期到过去时间应 422");
    let future2 = rfc3339_secs(chrono::Utc::now() + chrono::Duration::hours(5));
    let (st, v) = admin_json(
        &c,
        "PUT",
        &format!("{base}/api/admin/posts/{sched_id}"),
        &token,
        Some(json!({"published_at": &future2})),
    )
    .await;
    assert_eq!(st, 200);
    assert_eq!(v["status"], "scheduled");
    assert_eq!(v["published_at"], future2);

    // scheduled → draft：允许（取消定时），published_at 清空
    let (st, v) = admin_json(
        &c,
        "PUT",
        &format!("{base}/api/admin/posts/{sched_id}"),
        &token,
        Some(json!({"status": "draft"})),
    )
    .await;
    assert_eq!(st, 200);
    assert_eq!(v["status"], "draft");
    assert!(v["published_at"].is_null(), "取消定时应清空计划时间");

    // draft → scheduled：缺时间 → 422；带未来时间 → 200
    let r = c
        .put(format!("{base}/api/admin/posts/{sched_id}"))
        .bearer_auth(&token)
        .json(&json!({"status": "scheduled"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 422, "draft→scheduled 必须带未来时间");
    assert_eq!(err_code(r).await, "validation_error");
    let (st, v) = admin_json(
        &c,
        "PUT",
        &format!("{base}/api/admin/posts/{sched_id}"),
        &token,
        Some(json!({"status": "scheduled", "published_at": &future2})),
    )
    .await;
    assert_eq!(st, 200);
    assert_eq!(v["status"], "scheduled");
    assert_eq!(v["published_at"], future2);

    // scheduled → published：允许（立即发布），published_at 改写为 now（不再是计划时间）
    let (st, v) = admin_json(
        &c,
        "PUT",
        &format!("{base}/api/admin/posts/{sched_id}"),
        &token,
        Some(json!({"status": "published"})),
    )
    .await;
    assert_eq!(st, 200);
    assert_eq!(v["status"], "published");
    let pa = v["published_at"].as_str().unwrap().to_string();
    assert!(
        pa <= rfc3339_secs(chrono::Utc::now()),
        "立即发布应把 published_at 改写为当前时间，实际 {pa}"
    );
    assert!(pa < future2, "不应保留未来的计划时间");
    // 立即可见（公开列表含它）
    assert!(public_slugs(&c, &base)
        .await
        .contains(&"sched-ok".to_string()));

    // 现有 draft→published 行为不变：首发布写 published_at
    let (st, v) = admin_json(
        &c,
        "POST",
        &format!("{base}/api/admin/posts"),
        &token,
        Some(json!({"title": "D1", "slug": "d-one", "content_md": "x", "status": "draft"})),
    )
    .await;
    assert_eq!(st, 201);
    assert!(v["published_at"].is_null());
    let d_id = v["id"].as_i64().unwrap();
    let (_, v) = admin_json(
        &c,
        "PUT",
        &format!("{base}/api/admin/posts/{d_id}"),
        &token,
        Some(json!({"status": "published"})),
    )
    .await;
    assert_eq!(v["status"], "published");
    assert!(v["published_at"].as_str().is_some());

    // 管理列表 scheduled 过滤：此时无 scheduled 文章
    let v = get_json_authed(
        &c,
        &format!("{base}/api/admin/posts?status=scheduled"),
        &token,
    )
    .await;
    assert_eq!(v["total"], 0);
    let v = get_json_authed(&c, &format!("{base}/api/admin/posts?status=all"), &token).await;
    // 3 示例 + sched-ok + pub-now + d-one
    assert_eq!(v["total"], 6);
}
