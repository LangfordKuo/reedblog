//! 相关文章推荐集成测试（契约「相关文章推荐」条款，2026-10-04 新增）：
//! - 打分：2 × 共享标签数 + (同分类 ? 1 : 0)——多标签 > 单标签+同分类 > 单标签 > 仅同分类
//! - 排除文章自身；无任何共享标签/分类 → 空数组（不硬塞热门文章回退）
//! - 草稿、未到点 scheduled 永不出现；到点 scheduled 正常参与
//! - 排序：score DESC, view_count DESC, published_at DESC, id DESC 全键覆盖
//! - limit：默认 5、上限 10、显式值截断、越界/非数字 → 422 validation_error
//! - 文章不存在/未公开可见 → 404 not_found（与详情接口同口径）
//! - 形状：非分页裸数组，条目复用 PostPublic 列表条目形状

use reedblog_backend::state::AppState;
use serde_json::{json, Value};
use std::path::Path;

/// 在 127.0.0.1 随机端口起真实服务（与 post_nav.rs 同款隔离：插件/主题目录进 tempdir）
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

/// 安装 + 登录，返回 Bearer token（安装注入 3 篇示例文章，与本文件夹具的
/// 分类/标签名互不重合——夹具断言只针对自建 slug 集合，不受示例干扰）
async fn setup_installed(c: &reqwest::Client, base: &str, dir: &Path) -> String {
    let r = c
        .post(format!("{base}/api/install"))
        .json(&json!({
            "db_type": "sqlite",
            "sqlite_path": dir.join("reedblog.db").to_str().unwrap(),
            "admin": {"username": "admin", "password": "secret123"},
            "site": {"title": "相关文章测试站", "subtitle": "副标题"}
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

fn rfc3339_secs(dt: chrono::DateTime<chrono::Utc>) -> String {
    dt.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
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

async fn create_category(c: &reqwest::Client, base: &str, token: &str, name: &str) -> i64 {
    let (st, v) = admin_json(
        c,
        "POST",
        &format!("{base}/api/admin/categories"),
        token,
        Some(json!({"name": name})),
    )
    .await;
    assert_eq!(st, 201, "建分类失败: {v}");
    v["id"].as_i64().unwrap()
}

async fn create_tag(c: &reqwest::Client, base: &str, token: &str, name: &str) -> i64 {
    let (st, v) = admin_json(
        c,
        "POST",
        &format!("{base}/api/admin/tags"),
        token,
        Some(json!({"name": name})),
    )
    .await;
    assert_eq!(st, 201, "建标签失败: {v}");
    v["id"].as_i64().unwrap()
}

/// 建一篇文章（管理接口），返回 id；body 需含 title/slug/content_md/status
async fn create_post(c: &reqwest::Client, base: &str, token: &str, body: Value) -> i64 {
    let (st, v) = admin_json(
        c,
        "POST",
        &format!("{base}/api/admin/posts"),
        token,
        Some(body),
    )
    .await;
    assert_eq!(st, 201, "建文章失败: {v}");
    v["id"].as_i64().unwrap()
}

/// GET 相关文章接口：断言 200 并返回裸数组
async fn related(c: &reqwest::Client, base: &str, slug: &str, query: &str) -> Vec<Value> {
    let (st, v) = related_status(c, base, slug, query).await;
    assert_eq!(st, 200, "related({slug}{query}) 应为 200，实际 {st}: {v}");
    v.as_array().expect("相关文章应为裸数组").clone()
}

/// GET 相关文章接口：返回 (状态码, JSON)
async fn related_status(c: &reqwest::Client, base: &str, slug: &str, query: &str) -> (u16, Value) {
    let r = c
        .get(format!("{base}/api/posts/{slug}/related{query}"))
        .send()
        .await
        .unwrap();
    let status = r.status().as_u16();
    (status, r.json::<Value>().await.unwrap())
}

/// 结果条目的 slug 序列
fn slugs(items: &[Value]) -> Vec<String> {
    items
        .iter()
        .map(|i| i["slug"].as_str().unwrap().to_string())
        .collect()
}

/// 直接改库：固定 published_at（API 写不进去的时间数据）
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

/// 直接改库：固定 view_count（造同分排序键）
async fn db_set_view_count(url: &str, post_id: i64, n: i64) {
    sqlx::any::install_default_drivers();
    let pool = sqlx::AnyPool::connect(url).await.unwrap();
    sqlx::query("UPDATE posts SET view_count = ? WHERE id = ?")
        .bind(n)
        .bind(post_id)
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;
}

// ---------- 1. 打分：共享标签数 × 2 + 同分类加分；排除自身 ----------

#[tokio::test(flavor = "multi_thread")]
async fn scoring_shared_tags_then_category() {
    let tmp = tempfile::tempdir().unwrap();
    let base = spawn_server(tmp.path().join("config.toml").to_str().unwrap()).await;
    let c = reqwest::Client::new();
    let token = setup_installed(&c, &base, tmp.path()).await;

    let cat_a = create_category(&c, &base, &token, "评分类 A").await;
    let cat_b = create_category(&c, &base, &token, "评分类 B").await;
    let t1 = create_tag(&c, &base, &token, "评分 T1").await;
    let t2 = create_tag(&c, &base, &token, "评分 T2").await;
    let t3 = create_tag(&c, &base, &token, "评分 T3").await;
    let t4 = create_tag(&c, &base, &token, "评分 T4").await;

    // 目标：分类 A，标签 T1/T2/T3
    create_post(
        &c,
        &base,
        &token,
        json!({"title": "评分目标", "slug": "score-target", "content_md": "x",
               "status": "published", "category_id": cat_a, "tag_ids": [t1, t2, t3]}),
    )
    .await;
    // B：共享 2 标签（T1,T2）→ 2×2 = 4
    create_post(
        &c,
        &base,
        &token,
        json!({"title": "评分 B", "slug": "score-b", "content_md": "x",
               "status": "published", "category_id": cat_b, "tag_ids": [t1, t2]}),
    )
    .await;
    // C：共享 1 标签（T1）+ 同分类 A → 2+1 = 3
    create_post(
        &c,
        &base,
        &token,
        json!({"title": "评分 C", "slug": "score-c", "content_md": "x",
               "status": "published", "category_id": cat_a, "tag_ids": [t1]}),
    )
    .await;
    // D：共享 1 标签（T3）、异分类 → 2
    create_post(
        &c,
        &base,
        &token,
        json!({"title": "评分 D", "slug": "score-d", "content_md": "x",
               "status": "published", "category_id": cat_b, "tag_ids": [t3]}),
    )
    .await;
    // E：无共享标签、仅同分类 A → 1
    create_post(
        &c,
        &base,
        &token,
        json!({"title": "评分 E", "slug": "score-e", "content_md": "x",
               "status": "published", "category_id": cat_a, "tag_ids": []}),
    )
    .await;
    // F：无共享标签（T4）、异分类 B → 不出现
    create_post(
        &c,
        &base,
        &token,
        json!({"title": "评分 F", "slug": "score-f", "content_md": "x",
               "status": "published", "category_id": cat_b, "tag_ids": [t4]}),
    )
    .await;

    let items = related(&c, &base, "score-target", "").await;
    assert_eq!(
        slugs(&items),
        vec!["score-b", "score-c", "score-d", "score-e"],
        "打分应为 4 > 3 > 2 > 1，且 F（无共享）不出现"
    );
    assert!(
        !slugs(&items).contains(&"score-target".to_string()),
        "不得包含文章自身"
    );

    // 形状：条目为 PostPublic（列表复用形状，前端无需新类型）
    let first = &items[0];
    assert_eq!(first["title"], "评分 B");
    for key in [
        "id",
        "title",
        "slug",
        "excerpt",
        "category",
        "tags",
        "published_at",
        "comment_count",
        "is_sticky",
        "view_count",
        "likes",
    ] {
        assert!(first.get(key).is_some(), "条目缺少 PostPublic 字段 {key}");
    }
    assert!(first["tags"].is_array());
    assert!(first.get("content_md").is_none(), "列表条目不含 content_md");
}

// ---------- 2. 同分排序：view_count DESC → published_at DESC → id DESC ----------

#[tokio::test(flavor = "multi_thread")]
async fn tie_break_view_count_published_at_id() {
    let tmp = tempfile::tempdir().unwrap();
    let cfg_path = tmp.path().join("config.toml");
    let base = spawn_server(cfg_path.to_str().unwrap()).await;
    let c = reqwest::Client::new();
    let token = setup_installed(&c, &base, tmp.path()).await;
    let db_url = sqlite_url_from_config(cfg_path.to_str().unwrap());

    let cat = create_category(&c, &base, &token, "并列分类").await;
    let tag = create_tag(&c, &base, &token, "并列标签").await;

    // 目标：与所有候选共享同一枚标签 → 候选分数一致（均为 2），只由次级键区分
    create_post(
        &c,
        &base,
        &token,
        json!({"title": "并列目标", "slug": "tie-target", "content_md": "x",
               "status": "published", "category_id": cat, "tag_ids": [tag]}),
    )
    .await;

    // 创建顺序即 id 顺序：low < a < b < date < view
    let mut ids = Vec::new();
    for slug in ["tie-low", "tie-a", "tie-b", "tie-date", "tie-view"] {
        ids.push(
            create_post(
                &c,
                &base,
                &token,
                json!({"title": slug, "slug": slug, "content_md": "x",
                       "status": "published", "tag_ids": [tag]}),
            )
            .await,
        );
    }
    let (low, a, b, date, view) = (ids[0], ids[1], ids[2], ids[3], ids[4]);

    // 先统一发布时间到同一时刻，再单独抬高 tie-date 的发布时间
    for id in [low, a, b, date, view] {
        db_set_published_at(&db_url, id, "2026-01-01T00:00:00Z").await;
    }
    db_set_published_at(&db_url, date, "2026-03-01T00:00:00Z").await;

    // 浏览量：tie-view 最高；其余相同（3）
    for id in [low, a, b, date] {
        db_set_view_count(&db_url, id, 3).await;
    }
    db_set_view_count(&db_url, view, 10).await;

    let items = related(&c, &base, "tie-target", "").await;
    assert_eq!(
        slugs(&items),
        vec!["tie-view", "tie-date", "tie-b", "tie-a", "tie-low"],
        "同分应依次按 view_count DESC、published_at DESC、id DESC 排序"
    );
}

// ---------- 3. 可见性：草稿/未到点 scheduled 永不出现，到点 scheduled 正常参与 ----------

#[tokio::test(flavor = "multi_thread")]
async fn drafts_and_scheduled_visibility() {
    let tmp = tempfile::tempdir().unwrap();
    let cfg_path = tmp.path().join("config.toml");
    let base = spawn_server(cfg_path.to_str().unwrap()).await;
    let c = reqwest::Client::new();
    let token = setup_installed(&c, &base, tmp.path()).await;
    let db_url = sqlite_url_from_config(cfg_path.to_str().unwrap());

    let cat = create_category(&c, &base, &token, "可见分类").await;
    let tag = create_tag(&c, &base, &token, "可见标签").await;

    create_post(
        &c,
        &base,
        &token,
        json!({"title": "可见目标", "slug": "vis-target", "content_md": "x",
               "status": "published", "category_id": cat, "tag_ids": [tag]}),
    )
    .await;
    // 草稿：共享标签+同分类，若可见性失效会排在很前
    create_post(
        &c,
        &base,
        &token,
        json!({"title": "隐藏草稿", "slug": "vis-draft", "content_md": "x",
               "status": "draft", "category_id": cat, "tag_ids": [tag]}),
    )
    .await;
    // 未到点 scheduled
    let future = rfc3339_secs(chrono::Utc::now() + chrono::Duration::hours(1));
    create_post(
        &c,
        &base,
        &token,
        json!({"title": "未到点定时", "slug": "vis-future", "content_md": "x",
               "status": "scheduled", "published_at": &future,
               "category_id": cat, "tag_ids": [tag]}),
    )
    .await;
    // 到点 scheduled：先按未来时间创建（API 校验要求），再拨到过去模拟到点
    let due_future = rfc3339_secs(chrono::Utc::now() + chrono::Duration::hours(2));
    let due_id = create_post(
        &c,
        &base,
        &token,
        json!({"title": "已到点定时", "slug": "vis-due", "content_md": "x",
               "status": "scheduled", "published_at": &due_future,
               "category_id": cat, "tag_ids": [tag]}),
    )
    .await;
    // 普通 published
    create_post(
        &c,
        &base,
        &token,
        json!({"title": "正常发布", "slug": "vis-pub", "content_md": "x",
               "status": "published", "category_id": cat, "tag_ids": [tag]}),
    )
    .await;

    // 到点前：只有普通 published 出现
    let items = related(&c, &base, "vis-target", "").await;
    assert_eq!(
        slugs(&items),
        vec!["vis-pub"],
        "草稿与未到点 scheduled 不应出现"
    );

    // 拨到过去 → 到点可见，按 published_at DESC 排在正常发布之后
    db_set_published_at(&db_url, due_id, "2020-01-01T00:00:00Z").await;
    let items = related(&c, &base, "vis-target", "").await;
    assert_eq!(
        slugs(&items),
        vec!["vis-pub", "vis-due"],
        "到点 scheduled 应正常参与；草稿仍不出现"
    );

    // 不可见文章自身 404（与详情接口同口径）
    for slug in ["vis-draft", "vis-future"] {
        let (st, v) = related_status(&c, &base, slug, "").await;
        assert_eq!(st, 404, "{slug} 不可见时 related 应 404: {v}");
        assert_eq!(v["error"]["code"], "not_found");
    }
}

// ---------- 4. 无共享标签/分类 → 空数组（不硬塞热门文章） ----------

#[tokio::test(flavor = "multi_thread")]
async fn no_shared_tags_or_category_returns_empty() {
    let tmp = tempfile::tempdir().unwrap();
    let base = spawn_server(tmp.path().join("config.toml").to_str().unwrap()).await;
    let c = reqwest::Client::new();
    let token = setup_installed(&c, &base, tmp.path()).await;

    let cat = create_category(&c, &base, &token, "孤岛分类").await;
    let tag = create_tag(&c, &base, &token, "孤岛标签").await;

    // 有分类有标签，但全站没有任何文章共享（示例文章的分类/标签均不同名）
    create_post(
        &c,
        &base,
        &token,
        json!({"title": "孤岛文章", "slug": "island-post", "content_md": "x",
               "status": "published", "category_id": cat, "tag_ids": [tag]}),
    )
    .await;
    // 无分类、无标签
    create_post(
        &c,
        &base,
        &token,
        json!({"title": "光杆文章", "slug": "bare-post", "content_md": "x",
               "status": "published", "tag_ids": []}),
    )
    .await;

    // 热门文章（浏览量高）也不能被硬塞进来
    let hot = create_post(
        &c,
        &base,
        &token,
        json!({"title": "热门但无关", "slug": "hot-unrelated", "content_md": "x",
               "status": "published", "tag_ids": []}),
    )
    .await;
    db_set_view_count(
        &sqlite_url_from_config(tmp.path().join("config.toml").to_str().unwrap()),
        hot,
        9999,
    )
    .await;

    for slug in ["island-post", "bare-post"] {
        let items = related(&c, &base, slug, "").await;
        assert!(items.is_empty(), "{slug} 无共享时应为空数组: {items:?}");
    }
}

// ---------- 5. limit：默认 5、上限 10、截断、越界/非数字 422 ----------

#[tokio::test(flavor = "multi_thread")]
async fn limit_default_cap_and_validation() {
    let tmp = tempfile::tempdir().unwrap();
    let cfg_path = tmp.path().join("config.toml");
    let base = spawn_server(cfg_path.to_str().unwrap()).await;
    let c = reqwest::Client::new();
    let token = setup_installed(&c, &base, tmp.path()).await;
    let db_url = sqlite_url_from_config(cfg_path.to_str().unwrap());

    let tag = create_tag(&c, &base, &token, "限量标签").await;
    let tag2 = create_tag(&c, &base, &token, "限量标签2").await;

    // 目标带两枚标签：下面 limit-top 与它共享 2 枚（score 4），其余只共享 1 枚（score 2）
    create_post(
        &c,
        &base,
        &token,
        json!({"title": "限量目标", "slug": "limit-target", "content_md": "x",
               "status": "published", "tag_ids": [tag, tag2]}),
    )
    .await;
    // 头名：共享 2 枚标签 → score 4
    create_post(
        &c,
        &base,
        &token,
        json!({"title": "限量头名", "slug": "limit-top", "content_md": "x",
               "status": "published", "tag_ids": [tag, tag2]}),
    )
    .await;
    // 其余 7 篇：各共享 1 枚标签（score 2），浏览量 1..7 决定同分顺序
    let mut ids = Vec::new();
    for k in 1..=7 {
        ids.push(
            create_post(
                &c,
                &base,
                &token,
                json!({"title": format!("限量 {k}"), "slug": format!("limit-{k}"),
                       "content_md": "x", "status": "published", "tag_ids": [tag]}),
            )
            .await,
        );
    }
    for (i, id) in ids.iter().enumerate() {
        db_set_view_count(&db_url, *id, (i + 1) as i64).await;
    }
    // 期望全序：top, 7, 6, 5, 4, 3, 2, 1（共 8 条）
    let full: Vec<String> = std::iter::once("limit-top".to_string())
        .chain((1..=7).rev().map(|k| format!("limit-{k}")))
        .collect();

    // 缺省 limit=5：取前 5
    let items = related(&c, &base, "limit-target", "").await;
    assert_eq!(slugs(&items), full[..5].to_vec(), "limit 缺省应为 5");

    // 显式值截断
    let items = related(&c, &base, "limit-target", "?limit=1").await;
    assert_eq!(slugs(&items), full[..1].to_vec());
    let items = related(&c, &base, "limit-target", "?limit=3").await;
    assert_eq!(slugs(&items), full[..3].to_vec());
    // 上限 10：候选仅 8 条 → 全部返回（同时验证 10 合法）
    let items = related(&c, &base, "limit-target", "?limit=10").await;
    assert_eq!(slugs(&items), full, "limit=10 应合法且返回全部候选");
    // 恰好等于候选数
    let items = related(&c, &base, "limit-target", "?limit=8").await;
    assert_eq!(slugs(&items), full);
    // 首尾空白合法
    let items = related(&c, &base, "limit-target", "?limit=%205%20").await;
    assert_eq!(slugs(&items), full[..5].to_vec(), "带空白应合法");

    // 越界/非数字 → 422 validation_error
    for q in [
        "?limit=0",
        "?limit=11",
        "?limit=-1",
        "?limit=abc",
        "?limit=2.5",
        "?limit=",
    ] {
        let (st, v) = related_status(&c, &base, "limit-target", q).await;
        assert_eq!(st, 422, "{q} 应 422: {v}");
        assert_eq!(v["error"]["code"], "validation_error", "{q}");
    }
}

// ---------- 6. 文章不存在 → 404 not_found ----------

#[tokio::test(flavor = "multi_thread")]
async fn missing_post_returns_404() {
    let tmp = tempfile::tempdir().unwrap();
    let base = spawn_server(tmp.path().join("config.toml").to_str().unwrap()).await;
    let c = reqwest::Client::new();
    setup_installed(&c, &base, tmp.path()).await;

    let (st, v) = related_status(&c, &base, "no-such-post", "").await;
    assert_eq!(st, 404, "不存在的文章应 404: {v}");
    assert_eq!(v["error"]["code"], "not_found");
}
