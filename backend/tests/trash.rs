//! 文章回收站（软删除）集成测试（api-contract.md「文章回收站」条款，2026-10-04 新增）：
//! - 软删后：公开列表/详情/搜索/归档/标签与分类计数/RSS/sitemap/相关文章/上一篇下一篇
//!   全部不再出现该文章（404 或不计入）；评论端点与点赞三接口 404；站点统计 post_count 减一
//! - 后台列表（含 status 筛选）不含；trash 列表含且带 deleted_at；回收站中单条 GET/PUT/
//!   sticky/修订历史 → 404；软删期间同 slug 新建 → 409
//! - 软删不动评论/点赞/修订（DB 直查仍在），恢复后原样还在且公开路径重新可见；
//!   恢复只清 deleted_at（内容/状态/置顶/发布时间/updated_at 一律不变）
//! - purge 仅限回收站文章（不在回收站 → 404），事务内清理 likes/评论/修订/标签关联与文章行，
//!   slug 随之释放（可新建同名）
//! - scheduled 文章进回收站后即使到点也不可见（直接改库把 published_at 拨到过去模拟时间流逝），
//!   恢复后到点即恢复可见
//! - 三个新端点无鉴权 401；DELETE 已在回收站的 → 404、restore/purge 不在回收站的 → 404

use reedblog_backend::state::AppState;
use serde_json::{json, Value};
use sqlx::Row;
use std::path::Path;

/// 反滥用限流（契约「反滥用」：同 IP + 同目标 60 秒 1 条）生效后，测试中连续发评论
/// 需模拟不同访客来源——每个请求分配唯一 XFF，避免命中限流返回 429。
fn visitor_ip() -> String {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static N: AtomicUsize = AtomicUsize::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    format!("10.{}.{}.{}", (n / 62500) % 250, (n / 250) % 250, n % 250)
}

/// 在 127.0.0.1 随机端口起真实服务（插件/主题目录隔离到 tempdir）
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

/// 安装 + 登录，返回 Bearer token（安装会注入示例内容，本文件只操作自建文章）
async fn setup_installed(c: &reqwest::Client, base: &str, dir: &Path) -> String {
    let r = c
        .post(format!("{base}/api/install"))
        .json(&json!({
            "db_type": "sqlite",
            "sqlite_path": dir.join("reedblog.db").to_str().unwrap(),
            "admin": {"username": "admin", "password": "secret123"},
            "site": {"title": "回收站测试博客"}
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

/// 通用 JSON 请求（token=None 模拟未登录；204 等无 JSON 响应体回 Value::Null）
async fn json_req(
    c: &reqwest::Client,
    method: &str,
    url: &str,
    token: Option<&str>,
    body: Option<Value>,
) -> (u16, Value) {
    let mut req = c
        .request(method.parse().unwrap(), url)
        // 反滥用限流：每个请求模拟不同访客（否则连续评论会命中 60 秒 1 条返回 429）
        .header("x-forwarded-for", visitor_ip());
    if let Some(t) = token {
        req = req.bearer_auth(t);
    }
    if let Some(b) = body {
        req = req.json(&b);
    }
    let r = req.send().await.unwrap();
    let status = r.status().as_u16();
    let v = r.json::<Value>().await.unwrap_or(Value::Null);
    (status, v)
}

/// 文本请求（RSS/sitemap：XML 响应）
async fn text_req(c: &reqwest::Client, url: &str) -> (u16, String) {
    let r = c.get(url).send().await.unwrap();
    let status = r.status().as_u16();
    (status, r.text().await.unwrap_or_default())
}

/// 创建文章（断言 201），返回 PostAdmin JSON
async fn create_post(c: &reqwest::Client, base: &str, token: &str, body: Value) -> Value {
    let (status, v) = json_req(
        c,
        "POST",
        &format!("{base}/api/admin/posts"),
        Some(token),
        Some(body),
    )
    .await;
    assert_eq!(status, 201, "创建文章应 201: {v}");
    v
}

/// 分页响应的 slug 列表
fn slugs_of(page: &Value) -> Vec<String> {
    page["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["slug"].as_str().unwrap().to_string())
        .collect()
}

/// 裸数组响应（相关文章）的 slug 列表
fn slugs_of_arr(v: &Value) -> Vec<String> {
    v.as_array()
        .unwrap_or_else(|| panic!("应为裸数组: {v}"))
        .iter()
        .map(|i| i["slug"].as_str().unwrap().to_string())
        .collect()
}

/// 分页响应的 id 列表
fn ids_of(page: &Value) -> Vec<i64> {
    page["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["id"].as_i64().unwrap())
        .collect()
}

/// 归档计数总和
fn archive_sum(v: &Value) -> i64 {
    v.as_array()
        .unwrap()
        .iter()
        .map(|e| e["count"].as_i64().unwrap())
        .sum()
}

/// 按名称取标签/分类的 post_count（找不到 → -1，便于断言失败定位）
fn named_count(v: &Value, name: &str) -> i64 {
    v.as_array()
        .unwrap()
        .iter()
        .find(|e| e["name"] == name)
        .map(|e| e["post_count"].as_i64().unwrap())
        .unwrap_or(-1)
}

/// 安装写入的 config.toml → SQLite 连接 URL（复用 Config::db_url 的路径编码逻辑）
fn sqlite_url_from_config(config_path: &str) -> String {
    reedblog_backend::config::Config::load(Path::new(config_path))
        .expect("config.toml 应已由安装流程写入")
        .db_url()
        .expect("db_type 应为 sqlite")
}

/// 直连 SQLite 数 post_likes/post_revisions/post_tags 等按 post_id 关联的行数
async fn db_count_by_post(url: &str, table: &str, post_id: i64) -> i64 {
    sqlx::any::install_default_drivers();
    let pool = sqlx::AnyPool::connect(url).await.unwrap();
    let sql = format!("SELECT COUNT(*) FROM {table} WHERE post_id = ?");
    let count: i64 = sqlx::query(&sql)
        .bind(post_id)
        .fetch_one(&pool)
        .await
        .unwrap()
        .get(0);
    pool.close().await;
    count
}

/// 直连 SQLite 数某文章的文章评论行数（comments.post_id 为通用目标 id，须限定 target_type）
async fn db_count_comments(url: &str, post_id: i64) -> i64 {
    sqlx::any::install_default_drivers();
    let pool = sqlx::AnyPool::connect(url).await.unwrap();
    let count: i64 =
        sqlx::query("SELECT COUNT(*) FROM comments WHERE target_type = 'post' AND post_id = ?")
            .bind(post_id)
            .fetch_one(&pool)
            .await
            .unwrap()
            .get(0);
    pool.close().await;
    count
}

/// 直连 SQLite 数 posts 行（purge 后应为 0）
async fn db_count_posts(url: &str, post_id: i64) -> i64 {
    sqlx::any::install_default_drivers();
    let pool = sqlx::AnyPool::connect(url).await.unwrap();
    let count: i64 = sqlx::query("SELECT COUNT(*) FROM posts WHERE id = ?")
        .bind(post_id)
        .fetch_one(&pool)
        .await
        .unwrap()
        .get(0);
    pool.close().await;
    count
}

/// 直连 SQLite 读 posts.deleted_at（软删后应为非 NULL 字符串）
async fn db_deleted_at(url: &str, post_id: i64) -> Option<String> {
    sqlx::any::install_default_drivers();
    let pool = sqlx::AnyPool::connect(url).await.unwrap();
    let row = sqlx::query("SELECT deleted_at FROM posts WHERE id = ?")
        .bind(post_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    let v = row
        .try_get::<Option<String>, _>("deleted_at")
        .unwrap_or(None);
    pool.close().await;
    v
}

/// 直接改库模拟时间流逝（契约测试：把 planned scheduled 的 published_at 拨到过去）
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

// ---------- 1. 软删：全部公开读路径消失、后台排除、数据保留；恢复后原样复活 ----------

#[tokio::test(flavor = "multi_thread")]
async fn soft_delete_hides_everywhere_and_restore_revives() {
    let tmp = tempfile::tempdir().unwrap();
    let config_path = tmp.path().join("config.toml");
    let base = spawn_server(config_path.to_str().unwrap()).await;
    let c = reqwest::Client::new();
    let token = setup_installed(&c, &base, tmp.path()).await;
    let db_url = sqlite_url_from_config(config_path.to_str().unwrap());

    // 分类/标签 + 三篇同分类同标签文章（P0→P1→P2 形成相邻链；P1 为目标）
    let (_, cat) = json_req(
        &c,
        "POST",
        &format!("{base}/api/admin/categories"),
        Some(&token),
        Some(json!({"name": "回收分类"})),
    )
    .await;
    let cat_id = cat["id"].as_i64().unwrap();
    let (_, tag) = json_req(
        &c,
        "POST",
        &format!("{base}/api/admin/tags"),
        Some(&token),
        Some(json!({"name": "回收标签"})),
    )
    .await;
    let tag_id = tag["id"].as_i64().unwrap();

    let mk = |title: &str, slug: &str, content: &str| {
        json!({"title": title, "slug": slug, "content_md": content, "status": "published",
               "category_id": cat_id, "tag_ids": [tag_id]})
    };
    create_post(&c, &base, &token, mk("回收 P0 基准", "trash-p0", "P0 正文")).await;
    let p1 = create_post(
        &c,
        &base,
        &token,
        json!({"title": "回收 P1 目标", "slug": "trash-p1", "content_md": "P1 正文含 回收专属词XYZ 结尾",
               "status": "published", "category_id": cat_id, "tag_ids": [tag_id], "is_sticky": true}),
    )
    .await;
    let p1_id = p1["id"].as_i64().unwrap();
    create_post(&c, &base, &token, mk("回收 P2 后续", "trash-p2", "P2 正文")).await;

    // P1 上：顶级评论 + 回复、点赞、一次内容修订
    let (s, comment) = json_req(
        &c,
        "POST",
        &format!("{base}/api/posts/trash-p1/comments"),
        None,
        Some(json!({"author_name": "回收访客", "content": "回收评论一"})),
    )
    .await;
    assert_eq!(s, 201, "评论创建应 201: {comment}");
    let cid = comment["id"].as_i64().unwrap();
    let (s, _) = json_req(
        &c,
        "POST",
        &format!("{base}/api/posts/trash-p1/comments"),
        None,
        Some(json!({"author_name": "回收访客", "content": "回收回复一", "parent_id": cid})),
    )
    .await;
    assert_eq!(s, 201);
    let (s, like) = json_req(
        &c,
        "POST",
        &format!("{base}/api/posts/trash-p1/like"),
        None,
        Some(json!({"liker_key": "trash-liker"})),
    )
    .await;
    assert_eq!(s, 200);
    assert_eq!(like["likes"], 1);
    let (s, _) = json_req(
        &c,
        "PUT",
        &format!("{base}/api/admin/posts/{p1_id}"),
        Some(&token),
        Some(json!({"title": "回收 P1 目标（改）"})),
    )
    .await;
    assert_eq!(s, 200, "内容修订应保存成功");
    assert_eq!(db_count_by_post(&db_url, "post_revisions", p1_id).await, 2);

    // 软删前快照（取管理形状；恢复后必须逐字段不变）
    let (s, before) = json_req(
        &c,
        "GET",
        &format!("{base}/api/admin/posts/{p1_id}"),
        Some(&token),
        None,
    )
    .await;
    assert_eq!(s, 200, "软删前管理详情应 200: {before}");
    assert_eq!(before["status"], "published");
    assert_eq!(before["is_sticky"], true);
    assert!(
        before["deleted_at"].is_null(),
        "正常文章 deleted_at 应为 null"
    );
    let before_published_at = before["published_at"].clone();
    let before_updated_at = before["updated_at"].clone();
    assert_eq!(before["category_id"].as_i64().unwrap(), cat_id);
    assert_eq!(before["slug"], "trash-p1");

    let (_, list0) = json_req(
        &c,
        "GET",
        &format!("{base}/api/posts?per_page=100"),
        None,
        None,
    )
    .await;
    let total0 = list0["total"].as_i64().unwrap();
    let (_, arch0) = json_req(&c, "GET", &format!("{base}/api/archive"), None, None).await;
    let archive0 = archive_sum(&arch0);
    // 标签/分类各关联 P0/P1/P2 三篇
    let (_, tags0) = json_req(&c, "GET", &format!("{base}/api/tags"), None, None).await;
    assert_eq!(named_count(&tags0, "回收标签"), 3);
    let (_, cats0) = json_req(&c, "GET", &format!("{base}/api/categories"), None, None).await;
    assert_eq!(named_count(&cats0, "回收分类"), 3);
    let (_, stats0) = json_req(&c, "GET", &format!("{base}/api/site/stats"), None, None).await;
    let post_count0 = stats0["post_count"].as_i64().unwrap();
    let search_url = format!(
        "{base}/api/search?q={}&per_page=100",
        urlencoding::encode("回收专属词XYZ")
    );
    let (_, search0) = json_req(&c, "GET", &search_url, None, None).await;
    assert_eq!(search0["total"], 1, "软删前搜索应命中: {search0}");
    let (_, rel0) = json_req(
        &c,
        "GET",
        &format!("{base}/api/posts/trash-p2/related"),
        None,
        None,
    )
    .await;
    assert!(
        slugs_of_arr(&rel0).contains(&"trash-p1".to_string()),
        "软删前相关文章应含 P1: {rel0}"
    );
    let (s, feed0) = text_req(&c, &format!("{base}/api/feed.xml")).await;
    assert_eq!(s, 200);
    assert!(feed0.contains("trash-p1"), "软删前 RSS 应含 P1");
    let (s, map0) = text_req(&c, &format!("{base}/api/sitemap.xml")).await;
    assert_eq!(s, 200);
    assert!(map0.contains("/posts/trash-p1"), "软删前 sitemap 应含 P1");
    // 相邻链：P2 的前一篇是 P1、P0 的后一篇是 P1
    let (_, p2_before) =
        json_req(&c, "GET", &format!("{base}/api/posts/trash-p2"), None, None).await;
    assert_eq!(p2_before["prev_post"]["slug"], "trash-p1");
    let (_, p0_before) =
        json_req(&c, "GET", &format!("{base}/api/posts/trash-p0"), None, None).await;
    assert_eq!(p0_before["next_post"]["slug"], "trash-p1");

    // 三个新端点无鉴权 → 401 unauthorized
    for (method, path) in [
        ("GET", format!("{base}/api/admin/posts/trash")),
        ("POST", format!("{base}/api/admin/posts/{p1_id}/restore")),
        ("DELETE", format!("{base}/api/admin/posts/{p1_id}/purge")),
    ] {
        let (s, v) = json_req(&c, method, &path, None, None).await;
        assert_eq!(s, 401, "{method} {path} 未登录应 401: {v}");
        assert_eq!(v["error"]["code"], "unauthorized");
    }

    // ---- 移入回收站 ----
    let (s, _) = json_req(
        &c,
        "DELETE",
        &format!("{base}/api/admin/posts/{p1_id}"),
        Some(&token),
        None,
    )
    .await;
    assert_eq!(s, 204, "软删应 204");
    assert!(
        db_deleted_at(&db_url, p1_id).await.is_some(),
        "软删后 deleted_at 应为非 NULL"
    );
    // 已在回收站再删 → 404
    let (s, _) = json_req(
        &c,
        "DELETE",
        &format!("{base}/api/admin/posts/{p1_id}"),
        Some(&token),
        None,
    )
    .await;
    assert_eq!(s, 404, "重复软删应 404");

    // 后台：列表（含 status 筛选）不含；trash 列表含且带 deleted_at
    let (_, admin_all) = json_req(
        &c,
        "GET",
        &format!("{base}/api/admin/posts?status=all&per_page=100"),
        Some(&token),
        None,
    )
    .await;
    assert!(
        !ids_of(&admin_all).contains(&p1_id),
        "后台列表不得含回收站文章"
    );
    let (_, admin_pub) = json_req(
        &c,
        "GET",
        &format!("{base}/api/admin/posts?status=published&per_page=100"),
        Some(&token),
        None,
    )
    .await;
    assert!(!ids_of(&admin_pub).contains(&p1_id));
    let (s, trash) = json_req(
        &c,
        "GET",
        &format!("{base}/api/admin/posts/trash?per_page=100"),
        Some(&token),
        None,
    )
    .await;
    assert_eq!(s, 200, "回收站列表应 200: {trash}");
    assert_eq!(trash["total"], 1);
    assert!(ids_of(&trash).contains(&p1_id));
    let item = &trash["items"][0];
    assert!(
        item["deleted_at"].is_string(),
        "回收站条目应带 deleted_at: {item}"
    );
    assert_eq!(item["status"], "published");
    // 回收站中访问正常后台读写路径 → 404
    for (method, path) in [
        ("GET", format!("{base}/api/admin/posts/{p1_id}")),
        ("GET", format!("{base}/api/admin/posts/{p1_id}/revisions")),
    ] {
        let (s, _) = json_req(&c, method, &path, Some(&token), None).await;
        assert_eq!(s, 404, "回收站中 {method} {path} 应 404");
    }
    let (s, _) = json_req(
        &c,
        "PUT",
        &format!("{base}/api/admin/posts/{p1_id}"),
        Some(&token),
        Some(json!({"title": "不应生效"})),
    )
    .await;
    assert_eq!(s, 404, "回收站文章不可编辑");
    let (s, _) = json_req(
        &c,
        "PATCH",
        &format!("{base}/api/admin/posts/{p1_id}/sticky"),
        Some(&token),
        Some(json!({"is_sticky": false})),
    )
    .await;
    assert_eq!(s, 404, "回收站文章不可置顶");

    // 前台：详情/搜索/列表/归档/标签/分类/RSS/sitemap/相关文章/相邻 全部消失
    let (s, _) = json_req(&c, "GET", &format!("{base}/api/posts/trash-p1"), None, None).await;
    assert_eq!(s, 404, "回收站文章详情应 404");
    let (s, _) = json_req(
        &c,
        "GET",
        &format!("{base}/api/posts/trash-p1/comments"),
        None,
        None,
    )
    .await;
    assert_eq!(s, 404, "回收站文章评论列表应 404");
    let (s, _) = json_req(
        &c,
        "POST",
        &format!("{base}/api/posts/trash-p1/comments"),
        None,
        Some(json!({"author_name": "x", "content": "x"})),
    )
    .await;
    assert_eq!(s, 404, "回收站文章不可评论");
    let (s, _) = json_req(
        &c,
        "GET",
        &format!("{base}/api/posts/trash-p1/like?liker_key=trash-liker"),
        None,
        None,
    )
    .await;
    assert_eq!(s, 404, "回收站文章点赞状态应 404");
    let (s, _) = json_req(
        &c,
        "POST",
        &format!("{base}/api/posts/trash-p1/like"),
        None,
        Some(json!({"liker_key": "trash-liker"})),
    )
    .await;
    assert_eq!(s, 404, "回收站文章不可点赞");
    let (s, _) = json_req(
        &c,
        "DELETE",
        &format!("{base}/api/posts/trash-p1/like?liker_key=trash-liker"),
        None,
        None,
    )
    .await;
    assert_eq!(s, 404, "回收站文章不可取消点赞");

    let (_, list1) = json_req(
        &c,
        "GET",
        &format!("{base}/api/posts?per_page=100"),
        None,
        None,
    )
    .await;
    assert_eq!(list1["total"].as_i64().unwrap(), total0 - 1);
    assert!(!slugs_of(&list1).contains(&"trash-p1".to_string()));
    let (_, search1) = json_req(&c, "GET", &search_url, None, None).await;
    assert_eq!(search1["total"], 0, "回收站文章不应被搜到: {search1}");
    let (_, arch1) = json_req(&c, "GET", &format!("{base}/api/archive"), None, None).await;
    assert_eq!(archive_sum(&arch1), archive0 - 1, "归档计数应排除回收站");
    let (_, tags1) = json_req(&c, "GET", &format!("{base}/api/tags"), None, None).await;
    assert_eq!(named_count(&tags1, "回收标签"), 2, "标签计数应排除回收站");
    let (_, cats1) = json_req(&c, "GET", &format!("{base}/api/categories"), None, None).await;
    assert_eq!(named_count(&cats1, "回收分类"), 2, "分类计数应排除回收站");
    let (_, stats1) = json_req(&c, "GET", &format!("{base}/api/site/stats"), None, None).await;
    assert_eq!(stats1["post_count"].as_i64().unwrap(), post_count0 - 1);
    let (_, feed1) = text_req(&c, &format!("{base}/api/feed.xml")).await;
    assert!(!feed1.contains("trash-p1"), "回收站文章不应出现在 RSS");
    let (_, map1) = text_req(&c, &format!("{base}/api/sitemap.xml")).await;
    assert!(
        !map1.contains("/posts/trash-p1"),
        "回收站文章不应出现在 sitemap"
    );
    let (_, rel1) = json_req(
        &c,
        "GET",
        &format!("{base}/api/posts/trash-p2/related"),
        None,
        None,
    )
    .await;
    assert!(
        !slugs_of_arr(&rel1).contains(&"trash-p1".to_string()),
        "相关文章不得含回收站文章"
    );
    let (_, p2_after) =
        json_req(&c, "GET", &format!("{base}/api/posts/trash-p2"), None, None).await;
    assert_eq!(
        p2_after["prev_post"]["slug"], "trash-p0",
        "相邻项应跳过回收站文章"
    );
    let (_, p0_after) =
        json_req(&c, "GET", &format!("{base}/api/posts/trash-p0"), None, None).await;
    assert_eq!(
        p0_after["next_post"]["slug"], "trash-p2",
        "相邻项应跳过回收站文章"
    );

    // 软删期间数据原样保留（评论 2 / 点赞 1 / 修订 2 / 标签关联 1）
    assert_eq!(db_count_comments(&db_url, p1_id).await, 2);
    assert_eq!(db_count_by_post(&db_url, "post_likes", p1_id).await, 1);
    assert_eq!(db_count_by_post(&db_url, "post_revisions", p1_id).await, 2);
    assert_eq!(db_count_by_post(&db_url, "post_tags", p1_id).await, 1);

    // slug 仍被占用：软删期间新建同 slug → 409 slug_taken
    let (s, v) = json_req(
        &c,
        "POST",
        &format!("{base}/api/admin/posts"),
        Some(&token),
        Some(
            json!({"title": "冒名顶替", "slug": "trash-p1", "content_md": "x", "status": "draft"}),
        ),
    )
    .await;
    assert_eq!(s, 409, "软删期间同 slug 新建应 409: {v}");
    assert_eq!(v["error"]["code"], "slug_taken");

    // ---- 恢复 ----
    let (s, restored) = json_req(
        &c,
        "POST",
        &format!("{base}/api/admin/posts/{p1_id}/restore"),
        Some(&token),
        None,
    )
    .await;
    assert_eq!(s, 200, "恢复应 200: {restored}");
    assert!(
        restored["deleted_at"].is_null(),
        "恢复后 deleted_at 应为 null"
    );
    // 只清 deleted_at：内容/状态/置顶/发布时间/分类/标签/updated_at 全不变
    assert_eq!(restored["title"], "回收 P1 目标（改）");
    assert_eq!(restored["content_md"], "P1 正文含 回收专属词XYZ 结尾");
    assert_eq!(restored["status"], "published");
    assert_eq!(restored["is_sticky"], true);
    assert_eq!(restored["published_at"], before_published_at);
    assert_eq!(
        restored["updated_at"], before_updated_at,
        "软删/恢复不得更新 updated_at"
    );
    assert_eq!(restored["category_id"].as_i64().unwrap(), cat_id);
    assert_eq!(restored["tag_ids"].as_array().unwrap().len(), 1);
    // 恢复后再恢复 → 404
    let (s, _) = json_req(
        &c,
        "POST",
        &format!("{base}/api/admin/posts/{p1_id}/restore"),
        Some(&token),
        None,
    )
    .await;
    assert_eq!(s, 404, "不在回收站的恢复应 404");

    // 恢复后：评论/点赞/修订原样还在，公开路径重新可见
    let (s, comments) = json_req(
        &c,
        "GET",
        &format!("{base}/api/posts/trash-p1/comments"),
        None,
        None,
    )
    .await;
    assert_eq!(s, 200, "恢复后评论列表应可访问");
    assert_eq!(
        comments.as_array().unwrap().len(),
        2,
        "评论与回复应原样还在"
    );
    let (_, like_after) = json_req(
        &c,
        "GET",
        &format!("{base}/api/posts/trash-p1/like?liker_key=trash-liker"),
        None,
        None,
    )
    .await;
    assert_eq!(like_after["likes"], 1, "点赞应原样还在");
    assert_eq!(like_after["liked"], true);
    let (_, rev_after) = json_req(
        &c,
        "GET",
        &format!("{base}/api/admin/posts/{p1_id}/revisions"),
        Some(&token),
        None,
    )
    .await;
    assert_eq!(rev_after.as_array().unwrap().len(), 2, "修订历史应原样还在");

    let (_, list2) = json_req(
        &c,
        "GET",
        &format!("{base}/api/posts?per_page=100"),
        None,
        None,
    )
    .await;
    assert_eq!(
        list2["total"].as_i64().unwrap(),
        total0,
        "恢复后列表计数还原"
    );
    assert!(slugs_of(&list2).contains(&"trash-p1".to_string()));
    let (_, search2) = json_req(&c, "GET", &search_url, None, None).await;
    assert_eq!(search2["total"], 1, "恢复后搜索重新命中");
    let (_, tags2) = json_req(&c, "GET", &format!("{base}/api/tags"), None, None).await;
    assert_eq!(named_count(&tags2, "回收标签"), 3);
    let (_, stats2) = json_req(&c, "GET", &format!("{base}/api/site/stats"), None, None).await;
    assert_eq!(stats2["post_count"].as_i64().unwrap(), post_count0);
    let (_, feed2) = text_req(&c, &format!("{base}/api/feed.xml")).await;
    assert!(feed2.contains("trash-p1"), "恢复后 RSS 重新包含");
    let (_, map2) = text_req(&c, &format!("{base}/api/sitemap.xml")).await;
    assert!(map2.contains("/posts/trash-p1"), "恢复后 sitemap 重新包含");
    let (_, p2_back) = json_req(&c, "GET", &format!("{base}/api/posts/trash-p2"), None, None).await;
    assert_eq!(p2_back["prev_post"]["slug"], "trash-p1", "恢复后相邻链还原");
    let (_, trash_empty) = json_req(
        &c,
        "GET",
        &format!("{base}/api/admin/posts/trash?per_page=100"),
        Some(&token),
        None,
    )
    .await;
    assert_eq!(trash_empty["total"], 0, "恢复后回收站应为空");
}

// ---------- 2. purge：仅限回收站、级联清理、slug 释放 ----------

#[tokio::test(flavor = "multi_thread")]
async fn purge_only_from_trash_and_cascades() {
    let tmp = tempfile::tempdir().unwrap();
    let config_path = tmp.path().join("config.toml");
    let base = spawn_server(config_path.to_str().unwrap()).await;
    let c = reqwest::Client::new();
    let token = setup_installed(&c, &base, tmp.path()).await;
    let db_url = sqlite_url_from_config(config_path.to_str().unwrap());

    let (_, cat) = json_req(
        &c,
        "POST",
        &format!("{base}/api/admin/categories"),
        Some(&token),
        Some(json!({"name": "彻底删除分类"})),
    )
    .await;
    let cat_id = cat["id"].as_i64().unwrap();
    let (_, tag) = json_req(
        &c,
        "POST",
        &format!("{base}/api/admin/tags"),
        Some(&token),
        Some(json!({"name": "彻底删除标签"})),
    )
    .await;
    let tag_id = tag["id"].as_i64().unwrap();

    let post = create_post(
        &c,
        &base,
        &token,
        json!({"title": "彻底删除目标", "slug": "purge-p1", "content_md": "待彻底删除正文",
               "status": "published", "category_id": cat_id, "tag_ids": [tag_id]}),
    )
    .await;
    let id = post["id"].as_i64().unwrap();

    // 评论 + 回复 + 点赞 + 第二次修订
    let (s, comment) = json_req(
        &c,
        "POST",
        &format!("{base}/api/posts/purge-p1/comments"),
        None,
        Some(json!({"author_name": "访客", "content": "将随文章删除"})),
    )
    .await;
    assert_eq!(s, 201);
    let cid = comment["id"].as_i64().unwrap();
    let (s, _) = json_req(
        &c,
        "POST",
        &format!("{base}/api/posts/purge-p1/comments"),
        None,
        Some(json!({"author_name": "访客", "content": "回复也会删除", "parent_id": cid})),
    )
    .await;
    assert_eq!(s, 201);
    let (s, _) = json_req(
        &c,
        "POST",
        &format!("{base}/api/posts/purge-p1/like"),
        None,
        Some(json!({"liker_key": "purge-liker"})),
    )
    .await;
    assert_eq!(s, 200);
    let (s, _) = json_req(
        &c,
        "PUT",
        &format!("{base}/api/admin/posts/{id}"),
        Some(&token),
        Some(json!({"content_md": "第二版，仍待彻底删除"})),
    )
    .await;
    assert_eq!(s, 200);
    assert_eq!(db_count_by_post(&db_url, "post_revisions", id).await, 2);

    // 不在回收站：purge / restore 均 404（且文章不受影响）
    let (s, _) = json_req(
        &c,
        "DELETE",
        &format!("{base}/api/admin/posts/{id}/purge"),
        Some(&token),
        None,
    )
    .await;
    assert_eq!(s, 404, "未在回收站的文章不可彻底删除");
    let (s, _) = json_req(
        &c,
        "POST",
        &format!("{base}/api/admin/posts/{id}/restore"),
        Some(&token),
        None,
    )
    .await;
    assert_eq!(s, 404, "未在回收站的文章不可恢复");
    let (s, live) = json_req(&c, "GET", &format!("{base}/api/posts/purge-p1"), None, None).await;
    assert_eq!(s, 200, "404 的 purge/restore 不得影响文章: {live}");
    assert_eq!(live["content_md"], "第二版，仍待彻底删除");

    // 软删后 purge
    let (s, _) = json_req(
        &c,
        "DELETE",
        &format!("{base}/api/admin/posts/{id}"),
        Some(&token),
        None,
    )
    .await;
    assert_eq!(s, 204);
    let (s, _) = json_req(
        &c,
        "DELETE",
        &format!("{base}/api/admin/posts/{id}/purge"),
        Some(&token),
        None,
    )
    .await;
    assert_eq!(s, 204, "回收站文章彻底删除应 204");
    // 再 purge / 再软删 → 404
    let (s, _) = json_req(
        &c,
        "DELETE",
        &format!("{base}/api/admin/posts/{id}/purge"),
        Some(&token),
        None,
    )
    .await;
    assert_eq!(s, 404, "重复 purge 应 404");
    let (s, _) = json_req(
        &c,
        "DELETE",
        &format!("{base}/api/admin/posts/{id}"),
        Some(&token),
        None,
    )
    .await;
    assert_eq!(s, 404);

    // 级联清理：评论（含回复）/点赞/修订/标签关联/文章行全部消失
    assert_eq!(
        db_count_comments(&db_url, id).await,
        0,
        "purge 应清理评论与回复"
    );
    assert_eq!(
        db_count_by_post(&db_url, "post_likes", id).await,
        0,
        "purge 应清理点赞"
    );
    assert_eq!(
        db_count_by_post(&db_url, "post_revisions", id).await,
        0,
        "purge 应清理修订"
    );
    assert_eq!(
        db_count_by_post(&db_url, "post_tags", id).await,
        0,
        "purge 应清理标签关联"
    );
    assert_eq!(db_count_posts(&db_url, id).await, 0, "purge 应删除文章行");
    let (s, _) = json_req(
        &c,
        "GET",
        &format!("{base}/api/admin/posts/{id}"),
        Some(&token),
        None,
    )
    .await;
    assert_eq!(s, 404);
    let (s, _) = json_req(&c, "GET", &format!("{base}/api/posts/purge-p1"), None, None).await;
    assert_eq!(s, 404);
    let (_, trash) = json_req(
        &c,
        "GET",
        &format!("{base}/api/admin/posts/trash?per_page=100"),
        Some(&token),
        None,
    )
    .await;
    assert!(!ids_of(&trash).contains(&id), "purge 后不应仍在回收站列表");

    // slug 释放：可新建同名文章（409 → 201）
    let reused = create_post(
        &c,
        &base,
        &token,
        json!({"title": "复用 slug", "slug": "purge-p1", "content_md": "新正文", "status": "draft"}),
    )
    .await;
    assert_ne!(reused["id"].as_i64().unwrap(), id, "应是新文章行");

    // 不存在的文章 → 404
    let (s, _) = json_req(
        &c,
        "DELETE",
        &format!("{base}/api/admin/posts/999999/purge"),
        Some(&token),
        None,
    )
    .await;
    assert_eq!(s, 404);
}

// ---------- 3. scheduled 进回收站：到点也不可见；恢复后到点即可见 ----------

#[tokio::test(flavor = "multi_thread")]
async fn scheduled_in_trash_stays_hidden_even_when_due() {
    let tmp = tempfile::tempdir().unwrap();
    let config_path = tmp.path().join("config.toml");
    let base = spawn_server(config_path.to_str().unwrap()).await;
    let c = reqwest::Client::new();
    let token = setup_installed(&c, &base, tmp.path()).await;
    let db_url = sqlite_url_from_config(config_path.to_str().unwrap());

    let future = (chrono::Utc::now() + chrono::Duration::hours(1))
        .to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let post = create_post(
        &c,
        &base,
        &token,
        json!({"title": "定时回收目标", "slug": "trash-sched", "content_md": "定时回收专属词ABC",
               "status": "scheduled", "published_at": future}),
    )
    .await;
    let id = post["id"].as_i64().unwrap();

    let (s, _) = json_req(
        &c,
        "DELETE",
        &format!("{base}/api/admin/posts/{id}"),
        Some(&token),
        None,
    )
    .await;
    assert_eq!(s, 204, "scheduled 文章应可移入回收站");

    // 模拟时间流逝：计划时间拨到过去（若未删除，此刻应已公开可见）
    let past = (chrono::Utc::now() - chrono::Duration::minutes(1))
        .to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    db_set_published_at(&db_url, id, &past).await;

    // 到点但仍在回收站 → 所有公开路径不可见
    let (s, _) = json_req(
        &c,
        "GET",
        &format!("{base}/api/posts/trash-sched"),
        None,
        None,
    )
    .await;
    assert_eq!(s, 404, "回收站中的 scheduled 到点也不可见");
    let (_, list) = json_req(
        &c,
        "GET",
        &format!("{base}/api/posts?per_page=100"),
        None,
        None,
    )
    .await;
    assert!(!slugs_of(&list).contains(&"trash-sched".to_string()));
    let (_, search) = json_req(
        &c,
        "GET",
        &format!(
            "{base}/api/search?q={}&per_page=100",
            urlencoding::encode("定时回收专属词ABC")
        ),
        None,
        None,
    )
    .await;
    assert_eq!(search["total"], 0);
    let (_, feed) = text_req(&c, &format!("{base}/api/feed.xml")).await;
    assert!(!feed.contains("trash-sched"));
    let (_, map) = text_req(&c, &format!("{base}/api/sitemap.xml")).await;
    assert!(!map.contains("/posts/trash-sched"));
    // 回收站列表可见
    let (_, trash) = json_req(
        &c,
        "GET",
        &format!("{base}/api/admin/posts/trash?per_page=100"),
        Some(&token),
        None,
    )
    .await;
    assert!(ids_of(&trash).contains(&id));

    // 恢复后计划时间已到 → 立即可见（惰性可见性不变）
    let (s, restored) = json_req(
        &c,
        "POST",
        &format!("{base}/api/admin/posts/{id}/restore"),
        Some(&token),
        None,
    )
    .await;
    assert_eq!(s, 200);
    assert_eq!(restored["status"], "scheduled", "恢复不改状态");
    let (s, detail) = json_req(
        &c,
        "GET",
        &format!("{base}/api/posts/trash-sched"),
        None,
        None,
    )
    .await;
    assert_eq!(s, 200, "恢复到点后应立即公开可见: {detail}");
}
