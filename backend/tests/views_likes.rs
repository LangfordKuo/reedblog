//! 浏览量与点赞集成测试（api-contract.md「浏览量与点赞」条款）：
//! - 浏览量：公开详情命中 +1（响应含本次）；同 IP 60 分钟窗口去重（重复请求不二次计数）；
//!   列表不计数；带 Bearer 的请求不计数；爬虫/工具 UA 不计数
//! - hot 排序：view_count DESC, comment_count DESC, published_at DESC
//! - 点赞：POST/DELETE/GET 三接口、幂等、422 校验、404 可见性、likes 计数形状、
//!   未安装门禁 503、删文连带清理 post_likes
//! - 站点统计：stats.total_views = 所有文章浏览量和；管理形状含 view_count/likes

use serde_json::{json, Value};
use std::path::Path;

/// 反滥用限流（契约「反滥用」：同 IP + 同目标 60 秒 1 条）生效后，测试中连续发评论
/// 需模拟不同访客来源——每个请求分配唯一 XFF，避免命中限流返回 429。
fn visitor_ip() -> String {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static N: AtomicUsize = AtomicUsize::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    format!("10.{}.{}.{}", (n / 62500) % 250, (n / 250) % 250, n % 250)
}

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
        "site": {"title": "浏览点赞测试博客"}
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

/// 管理端创建文章，返回 (id, slug)
async fn create_post(
    c: &reqwest::Client,
    base: &str,
    token: &str,
    title: &str,
    status: &str,
) -> (i64, String) {
    let r = c
        .post(format!("{base}/api/admin/posts"))
        .bearer_auth(token)
        .json(&json!({
            "title": title, "slug": title, "content_md": "正文内容", "status": status
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 201, "创建文章 {title} 失败");
    let v: Value = r.json().await.unwrap();
    (
        v["id"].as_i64().unwrap(),
        v["slug"].as_str().unwrap().to_string(),
    )
}

/// GET 文章详情（可带 XFF / UA / Bearer），返回响应 JSON
async fn detail(
    c: &reqwest::Client,
    base: &str,
    slug: &str,
    xff: Option<&str>,
    ua: Option<&str>,
    bearer: Option<&str>,
) -> Value {
    let mut req = c.get(format!("{base}/api/posts/{slug}"));
    if let Some(v) = xff {
        req = req.header("x-forwarded-for", v);
    }
    if let Some(v) = ua {
        req = req.header("user-agent", v);
    }
    if let Some(t) = bearer {
        req = req.bearer_auth(t);
    }
    let r = req.send().await.unwrap();
    assert_eq!(r.status(), 200, "GET detail {slug}");
    r.json().await.unwrap()
}

/// 以不同 IP（XFF）访问详情 n 次，返回最后响应的 view_count
async fn bump_views(c: &reqwest::Client, base: &str, slug: &str, n: i64, ip_seed: i64) -> i64 {
    let mut v = 0;
    for i in 0..n {
        let body = detail(
            c,
            base,
            slug,
            Some(&format!("9.9.{}.{}", ip_seed, i)),
            Some("Mozilla/5.0 (Windows NT 10.0) TestBrowser/1.0"),
            None,
        )
        .await;
        v = body["view_count"].as_i64().unwrap();
    }
    v
}

// ---------- 1. 浏览量：详情命中 +1、同 IP 去重、列表/Bearer/爬虫不计数 ----------

#[tokio::test(flavor = "multi_thread")]
async fn view_count_increments_and_dedups() {
    let tmp = tempfile::tempdir().unwrap();
    let base = spawn_server(tmp.path().join("config.toml").to_str().unwrap()).await;
    let c = reqwest::Client::new();
    let token = setup_installed(&c, &base, tmp.path()).await;
    let (_id, slug) = create_post(&c, &base, &token, "view-post", "published").await;

    let browser_ua = "Mozilla/5.0 (Macintosh) AppleWebKit/537.36 Safari/537.36";

    // 首次命中 → +1，且响应中的 view_count 已含本次
    let v = detail(&c, &base, &slug, Some("1.1.1.1"), Some(browser_ua), None).await;
    assert_eq!(v["view_count"], 1, "详情命中应 +1（响应含本次）");
    assert_eq!(v["likes"], 0, "PostDetail 形状含 likes 字段");

    // 同一 IP（XFF 相同）60 分钟窗口内重复请求 → 不二次计数
    let v = detail(&c, &base, &slug, Some("1.1.1.1"), Some(browser_ua), None).await;
    assert_eq!(v["view_count"], 1, "同 IP 窗口内去重");
    let v = detail(&c, &base, &slug, Some("1.1.1.1"), Some(browser_ua), None).await;
    assert_eq!(v["view_count"], 1, "同 IP 第三次仍去重");

    // 不同 IP → 再计一次
    let v = detail(&c, &base, &slug, Some("2.2.2.2"), Some(browser_ua), None).await;
    assert_eq!(v["view_count"], 2, "不同 IP 应再计数");

    // 列表接口不计数（用新 IP 访问列表，再看详情：仍是新 IP 首访才 +1）
    let list: Value = c
        .get(format!("{base}/api/posts"))
        .header("x-forwarded-for", "3.3.3.3")
        .header("user-agent", browser_ua)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let item = list["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["slug"] == slug)
        .expect("列表应含该文章");
    assert_eq!(item["view_count"], 2, "列表不计数，且形状含 view_count");
    assert!(item.get("likes").is_some(), "PostPublic 列表形状含 likes");

    // 带 Bearer 的请求（后台预览）不计数
    let v = detail(
        &c,
        &base,
        &slug,
        Some("4.4.4.4"),
        Some(browser_ua),
        Some(&token),
    )
    .await;
    assert_eq!(v["view_count"], 2, "Bearer 请求不计数");

    // 爬虫/工具 UA 不计数
    for ua in [
        "Mozilla/5.0 (compatible; Googlebot/2.1; +http://www.google.com/bot.html)",
        "curl/8.4.0",
        "Mozilla/5.0 (compatible; Baiduspider/2.0)",
    ] {
        let v = detail(&c, &base, &slug, Some("5.5.5.5"), Some(ua), None).await;
        assert_eq!(v["view_count"], 2, "爬虫 UA 不计数: {ua}");
    }

    // 无 UA 头的正常请求照常计数（新 IP）
    let v = detail(&c, &base, &slug, Some("6.6.6.6"), None, None).await;
    assert_eq!(v["view_count"], 3, "无 UA 不视为爬虫");

    // 去重键含 post_id：同 IP 访问另一篇文章照常计数
    let (_id2, slug2) = create_post(&c, &base, &token, "view-post-2", "published").await;
    let v = detail(&c, &base, &slug2, Some("1.1.1.1"), Some(browser_ua), None).await;
    assert_eq!(v["view_count"], 1, "同 IP 不同文章分别计数");

    // 草稿详情 404，不产生计数副作用
    let (_id3, draft) = create_post(&c, &base, &token, "view-draft", "draft").await;
    let r = c
        .get(format!("{base}/api/posts/{draft}"))
        .header("x-forwarded-for", "7.7.7.7")
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 404);
}

// ---------- 2. hot 排序：view_count DESC, comment_count DESC ----------

#[tokio::test(flavor = "multi_thread")]
async fn hot_order_by_view_count_then_comments() {
    let tmp = tempfile::tempdir().unwrap();
    let base = spawn_server(tmp.path().join("config.toml").to_str().unwrap()).await;
    let c = reqwest::Client::new();
    let token = setup_installed(&c, &base, tmp.path()).await;

    let (_a, slug_a) = create_post(&c, &base, &token, "hot-a", "published").await;
    let (_b, slug_b) = create_post(&c, &base, &token, "hot-b", "published").await;
    let (_d, slug_d) = create_post(&c, &base, &token, "hot-d", "published").await;

    // A 有 2 条评论但 0 浏览；B 无评论但 2 次浏览 → hot 序 B 在 A 前（浏览量优先）
    for i in 0..2 {
        let r = c
            .post(format!("{base}/api/posts/{slug_a}/comments"))
            .header("x-forwarded-for", visitor_ip())
            .json(&json!({"author_name": "路人", "content": format!("评论 {i}")}))
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), 201);
    }
    assert_eq!(bump_views(&c, &base, &slug_b, 2, 10).await, 2);

    // D：0 浏览 0 评论
    let hot: Value = c
        .get(format!("{base}/api/posts?order=hot&per_page=100"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let slugs: Vec<&str> = hot["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["slug"].as_str().unwrap())
        .collect();
    let pos = |s: &str| slugs.iter().position(|x| *x == s).unwrap();
    assert!(
        pos(&slug_b) < pos(&slug_a),
        "浏览量高的 B 应排在评论多但 0 浏览的 A 之前: {slugs:?}"
    );
    assert!(
        pos(&slug_a) < pos(&slug_d),
        "浏览量同为 0 时评论多的 A 排在 D 之前（comment_count 次序）: {slugs:?}"
    );
}

// ---------- 3. 点赞三接口：点赞/取消/幂等/422/404/计数形状 ----------

#[tokio::test(flavor = "multi_thread")]
async fn like_unlike_idempotent_and_validation() {
    let tmp = tempfile::tempdir().unwrap();
    let base = spawn_server(tmp.path().join("config.toml").to_str().unwrap()).await;
    let c = reqwest::Client::new();
    let token = setup_installed(&c, &base, tmp.path()).await;
    let (_id, slug) = create_post(&c, &base, &token, "like-post", "published").await;
    let url = format!("{base}/api/posts/{slug}/like");
    let k1 = "11111111-1111-4111-8111-111111111111";
    let k2 = "22222222-2222-4222-8222-222222222222";

    // GET 初始状态：未赞
    let r = c.get(format!("{url}?liker_key={k1}")).send().await.unwrap();
    assert_eq!(r.status(), 200);
    assert_eq!(
        r.json::<Value>().await.unwrap(),
        json!({"likes": 0, "liked": false})
    );

    // POST 点赞 → {likes:1, liked:true}
    let r = c
        .post(&url)
        .json(&json!({"liker_key": k1}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    assert_eq!(
        r.json::<Value>().await.unwrap(),
        json!({"likes": 1, "liked": true})
    );

    // 重复点赞同 key → 幂等，不报错、likes 不涨
    let r = c
        .post(&url)
        .json(&json!({"liker_key": k1}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    assert_eq!(
        r.json::<Value>().await.unwrap(),
        json!({"likes": 1, "liked": true}),
        "重复点赞应幂等"
    );

    // GET：k1 已赞、k2 未赞
    let v: Value = c
        .get(format!("{url}?liker_key={k1}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(v, json!({"likes": 1, "liked": true}));
    let v: Value = c
        .get(format!("{url}?liker_key={k2}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(v, json!({"likes": 1, "liked": false}));

    // 第二个访客点赞 → likes=2
    let r = c
        .post(&url)
        .json(&json!({"liker_key": k2}))
        .send()
        .await
        .unwrap();
    assert_eq!(
        r.json::<Value>().await.unwrap(),
        json!({"likes": 2, "liked": true})
    );

    // likes 计数形状：列表 PostPublic 与详情 PostDetail 都带总数
    let list: Value = c
        .get(format!("{base}/api/posts?per_page=100"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let item = list["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["slug"] == slug)
        .unwrap();
    assert_eq!(item["likes"], 2, "列表 likes 应为子查询总数");
    let d: Value = c
        .get(format!("{base}/api/posts/{slug}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(d["likes"], 2, "详情 likes");

    // DELETE（query 带 key）取消点赞 → {likes:1, liked:false}
    let r = c
        .delete(format!("{url}?liker_key={k1}"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    assert_eq!(
        r.json::<Value>().await.unwrap(),
        json!({"likes": 1, "liked": false})
    );

    // 再 DELETE 同 key → 幂等（未点赞过不报错）
    let r = c
        .delete(format!("{url}?liker_key={k1}"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    assert_eq!(
        r.json::<Value>().await.unwrap(),
        json!({"likes": 1, "liked": false}),
        "取消不存在的点赞应幂等"
    );

    // DELETE（JSON body 带 key，无 query）同样可用
    let r = c
        .delete(&url)
        .json(&json!({"liker_key": k2}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    assert_eq!(
        r.json::<Value>().await.unwrap(),
        json!({"likes": 0, "liked": false})
    );

    // 422 校验：liker_key 缺失 / 空白 / 超长
    let r = c.post(&url).json(&json!({})).send().await.unwrap();
    assert_eq!(r.status(), 422, "缺 liker_key → 422");
    assert_eq!(err_code(r).await, "validation_error");
    let r = c
        .post(&url)
        .json(&json!({"liker_key": "   "}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 422, "空白 liker_key → 422");
    let r = c
        .post(&url)
        .json(&json!({"liker_key": "x".repeat(65)}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 422, "超长 liker_key → 422");
    // 恰好 64 字符合法
    let r = c
        .post(&url)
        .json(&json!({"liker_key": "y".repeat(64)}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200, "64 字符 liker_key 合法");
    let r = c.get(&url).send().await.unwrap();
    assert_eq!(r.status(), 422, "GET 缺 liker_key → 422");
    let r = c.delete(&url).send().await.unwrap();
    assert_eq!(r.status(), 422, "DELETE 缺 liker_key → 422");

    // 不存在 / 未公开可见（草稿）→ 404
    let r = c
        .post(format!("{base}/api/posts/nope/like"))
        .json(&json!({"liker_key": k1}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 404);
    assert_eq!(err_code(r).await, "not_found");
    let (_id2, draft) = create_post(&c, &base, &token, "like-draft", "draft").await;
    let r = c
        .post(format!("{base}/api/posts/{draft}/like"))
        .json(&json!({"liker_key": k1}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 404, "草稿不可点赞");
}

// ---------- 4. 门禁（未安装 503）与删文连带清理 ----------

#[tokio::test(flavor = "multi_thread")]
async fn like_gate_and_delete_cascade() {
    // 未安装：点赞三接口不进白名单 → 503 not_installed
    let tmp0 = tempfile::tempdir().unwrap();
    let base0 = spawn_server(tmp0.path().join("config.toml").to_str().unwrap()).await;
    let c = reqwest::Client::new();
    let r = c
        .post(format!("{base0}/api/posts/x/like"))
        .json(&json!({"liker_key": "k"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 503);
    assert_eq!(err_code(r).await, "not_installed");
    let r = c
        .get(format!("{base0}/api/posts/x/like?liker_key=k"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 503);
    let r = c
        .delete(format!("{base0}/api/posts/x/like?liker_key=k"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 503);

    // 已安装：点赞后删文（契约「文章回收站」：DELETE=软删，保留点赞；purge 才连带清理）
    let tmp = tempfile::tempdir().unwrap();
    let config_path = tmp.path().join("config.toml");
    let base = spawn_server(config_path.to_str().unwrap()).await;
    let token = setup_installed(&c, &base, tmp.path()).await;
    let (id, slug) = create_post(&c, &base, &token, "cascade-post", "published").await;
    let r = c
        .post(format!("{base}/api/posts/{slug}/like"))
        .json(&json!({"liker_key": "liker-a"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);

    // 管理形状含 view_count / likes（只读展示）
    let v: Value = c
        .get(format!("{base}/api/admin/posts/{id}"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(v["likes"], 1, "PostAdmin.likes");
    assert!(v.get("view_count").is_some(), "PostAdmin.view_count");

    // 移入回收站（软删）→ 204：点赞原样保留（恢复后还在）
    let r = c
        .delete(format!("{base}/api/admin/posts/{id}"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 204);

    // 直连同一 SQLite 库验证 post_likes 未被软删清理（connect_pool 幂等跑迁移）
    let cfg = reedblog_backend::config::Config::load(&config_path).unwrap();
    let pool = reedblog_backend::state::connect_pool("sqlite", &cfg.db_url().unwrap())
        .await
        .unwrap();
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM post_likes WHERE post_id = ?")
        .bind(id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(n, 1, "软删不得清理 post_likes 行");

    // 彻底删除（purge，仅限回收站）→ 204：post_likes 才连带清空
    let r = c
        .delete(format!("{base}/api/admin/posts/{id}/purge"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 204);
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM post_likes WHERE post_id = ?")
        .bind(id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(n, 0, "purge 应连带删除 post_likes 行");
}

// ---------- 5. stats.total_views ----------

#[tokio::test(flavor = "multi_thread")]
async fn stats_total_views() {
    let tmp = tempfile::tempdir().unwrap();
    let base = spawn_server(tmp.path().join("config.toml").to_str().unwrap()).await;
    let c = reqwest::Client::new();
    let token = setup_installed(&c, &base, tmp.path()).await;

    // 初始（示例文章 0 浏览）→ total_views = 0
    let stats: Value = c
        .get(format!("{base}/api/site/stats"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(stats["total_views"], 0);

    // 示例文章 axum-sqlx +2、自建文章 +1 → total_views = 3
    assert_eq!(bump_views(&c, &base, "axum-sqlx", 2, 20).await, 2);
    let (_id, slug) = create_post(&c, &base, &token, "stats-post", "published").await;
    assert_eq!(bump_views(&c, &base, &slug, 1, 30).await, 1);

    let stats: Value = c
        .get(format!("{base}/api/site/stats"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(stats["total_views"], 3, "total_views 应为所有文章浏览量和");
    // 既有字段不受影响
    assert!(stats["post_count"].as_i64().unwrap() >= 4);
    assert!(stats["installed_at"].as_str().unwrap().ends_with('Z'));
}
