//! 文章详情「上一篇/下一篇」集成测试（契约「文章上一篇/下一篇」条款，2026-10-04 新增）：
//! - 语义：prev = 发布时间更早的相邻文章、next = 更晚；相邻关系按纯时间序
//!   (published_at DESC, id DESC)，同秒发布按 id 稳定 tiebreak
//! - 边界：最新一篇 next=null、最老一篇 prev=null；中间文章两侧都有值且指向正确
//! - 可见性：草稿、未到点的 scheduled 不作相邻项；到点的 scheduled 正常参与
//! - 与置顶解耦：置顶改变列表页排序，但不影响任何文章的相邻关系
//! - 形状：相邻项只含 {title, slug}；PostPublic 列表不含这两个字段

use reedblog_backend::state::AppState;
use serde_json::{json, Value};
use std::path::Path;

/// 在 127.0.0.1 随机端口起真实服务（与 sticky_scheduled.rs 同款隔离：插件/主题目录进 tempdir）
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

/// 安装 + 登录，返回 Bearer token（安装注入 3 篇示例文章：
/// 欢迎使用 reedblog 2026-09-20 / 用 Axum 和 SQLx… 2026-08-15 / 周末随笔… 2026-07-08）
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

/// 直接改库构造 API 写不进去的时间数据（固定 published_at / 把 scheduled 拨到过去）
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

/// 详情响应中的相邻项 → (title, slug)；null → None
fn nav_of(v: &Value, field: &str) -> Option<(String, String)> {
    let n = v.get(field)?;
    if n.is_null() {
        return None;
    }
    Some((
        n["title"].as_str().unwrap().to_string(),
        n["slug"].as_str().unwrap().to_string(),
    ))
}

/// 公开列表（纯时间序样本：items[0]=最新、items[2]=最老）的 (title, slug) 三分组
fn samples(v: &Value) -> [(String, String); 3] {
    let items = v["items"].as_array().unwrap();
    assert_eq!(items.len(), 3, "示例文章应为 3 篇");
    // 列表 recent 序此时无置顶 = 纯时间序
    std::array::from_fn(|i| {
        (
            items[i]["title"].as_str().unwrap().to_string(),
            items[i]["slug"].as_str().unwrap().to_string(),
        )
    })
}

// ---------- 1. 中间篇两侧都有值 + 首末边界 ----------

#[tokio::test(flavor = "multi_thread")]
async fn middle_first_and_last_neighbors() {
    let tmp = tempfile::tempdir().unwrap();
    let base = spawn_server(tmp.path().join("config.toml").to_str().unwrap()).await;
    let c = reqwest::Client::new();
    setup_installed(&c, &base, tmp.path()).await;

    let list = get_json(&c, &format!("{base}/api/posts")).await;
    let [newest, middle, oldest] = samples(&list);

    // 列表形状不变：PostPublic 不含 prev_post/next_post
    assert!(list["items"][0].get("prev_post").is_none());
    assert!(list["items"][0].get("next_post").is_none());

    // 中间文章：两侧都有值；prev=更早（最老）、next=更晚（最新）
    let d = get_json(&c, &format!("{base}/api/posts/{}", middle.1)).await;
    assert_eq!(
        nav_of(&d, "prev_post"),
        Some(oldest.clone()),
        "prev 应为发布时间更早的最老一篇"
    );
    assert_eq!(
        nav_of(&d, "next_post"),
        Some(newest.clone()),
        "next 应为发布时间更晚的最新一篇"
    );
    // 相邻项只含 title/slug 两个键
    let obj = d["prev_post"].as_object().unwrap();
    assert_eq!(obj.len(), 2, "相邻项不应带其他字段: {obj:?}");
    assert!(obj.contains_key("title") && obj.contains_key("slug"));

    // 最新一篇：next=null（没有更晚的）、prev=次新
    let d = get_json(&c, &format!("{base}/api/posts/{}", newest.1)).await;
    assert!(d["next_post"].is_null(), "最新一篇 next 应为 null");
    assert_eq!(nav_of(&d, "prev_post"), Some(middle.clone()));

    // 最老一篇：prev=null（没有更早的）、next=次老
    let d = get_json(&c, &format!("{base}/api/posts/{}", oldest.1)).await;
    assert!(d["prev_post"].is_null(), "最老一篇 prev 应为 null");
    assert_eq!(nav_of(&d, "next_post"), Some(middle.clone()));
}

// ---------- 2. 草稿 / 定时发布：不可见者不作相邻项，到点者正常参与 ----------

#[tokio::test(flavor = "multi_thread")]
async fn drafts_and_scheduled_visibility_in_neighbors() {
    let tmp = tempfile::tempdir().unwrap();
    let cfg_path = tmp.path().join("config.toml");
    let base = spawn_server(cfg_path.to_str().unwrap()).await;
    let c = reqwest::Client::new();
    let token = setup_installed(&c, &base, tmp.path()).await;
    let db_url = sqlite_url_from_config(cfg_path.to_str().unwrap());

    // 草稿：published_at 为 null，永远不可见
    let (st, _) = admin_json(
        &c,
        "POST",
        &format!("{base}/api/admin/posts"),
        &token,
        Some(json!({
            "title": "隐藏草稿", "slug": "draft-hidden", "content_md": "x", "status": "draft"
        })),
    )
    .await;
    assert_eq!(st, 201);

    // 未到点的 scheduled：计划时间在未来 1 小时——若可见性失效它会成为「最新一篇」
    let future = rfc3339_secs(chrono::Utc::now() + chrono::Duration::hours(1));
    let (st, sched) = admin_json(
        &c,
        "POST",
        &format!("{base}/api/admin/posts"),
        &token,
        Some(json!({
            "title": "未到点定时", "slug": "sched-future", "content_md": "x",
            "status": "scheduled", "published_at": &future
        })),
    )
    .await;
    assert_eq!(st, 201);
    let sched_id = sched["id"].as_i64().unwrap();

    let list = get_json(&c, &format!("{base}/api/posts")).await;
    let [newest, middle, oldest] = samples(&list);
    assert_eq!(list["total"], 3, "草稿与未到点 scheduled 不进公开列表");

    // 最新一篇 next 仍为 null：未到点的 scheduled 若被算作相邻会顶到这里
    let d = get_json(&c, &format!("{base}/api/posts/{}", newest.1)).await;
    assert!(d["next_post"].is_null(), "未到点的 scheduled 不应作 next");
    // 三篇的相邻项都不含草稿/未到点定时
    for slug in [&newest.1, &middle.1, &oldest.1] {
        let d = get_json(&c, &format!("{base}/api/posts/{slug}")).await;
        for field in ["prev_post", "next_post"] {
            if let Some((_, s)) = nav_of(&d, field) {
                assert_ne!(s, "draft-hidden", "草稿不应作相邻项（{slug} 的 {field}）");
                assert_ne!(s, "sched-future", "未到点 scheduled 不应作相邻项（{slug} 的 {field}）");
            }
        }
    }
    // 草稿本身详情 404（不可见）
    let r = c
        .get(format!("{base}/api/posts/draft-hidden"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 404);

    // 模拟到点：直接把计划时间拨到 08-01（介于 08-15 与 07-08 之间），status 仍为 scheduled
    db_set_published_at(&db_url, sched_id, "2026-08-01T00:00:00Z").await;

    let list = get_json(&c, &format!("{base}/api/posts")).await;
    assert_eq!(list["total"], 4, "到点的 scheduled 应公开可见");
    // 时间序：最新(09-20) → 中间(08-15) → 到点(08-01) → 最老(07-08)
    let due = ("未到点定时".to_string(), "sched-future".to_string());
    let d = get_json(&c, &format!("{base}/api/posts/{}", middle.1)).await;
    assert_eq!(nav_of(&d, "prev_post"), Some(due.clone()), "到点的 scheduled 应作 prev");
    assert_eq!(nav_of(&d, "next_post"), Some(newest.clone()));
    let d = get_json(&c, &format!("{base}/api/posts/{}", oldest.1)).await;
    assert_eq!(nav_of(&d, "next_post"), Some(due.clone()));
    // 到点文章自身两侧：prev=最老、next=中间
    let d = get_json(&c, &format!("{base}/api/posts/sched-future")).await;
    assert_eq!(nav_of(&d, "prev_post"), Some(oldest.clone()));
    assert_eq!(nav_of(&d, "next_post"), Some(middle.clone()));
    // 草稿仍然缺席
    for slug in [&newest.1, &middle.1, &oldest.1, &due.1] {
        let d = get_json(&c, &format!("{base}/api/posts/{slug}")).await;
        for field in ["prev_post", "next_post"] {
            if let Some((_, s)) = nav_of(&d, field) {
                assert_ne!(s, "draft-hidden", "草稿不应作相邻项（{slug} 的 {field}）");
            }
        }
    }
}

// ---------- 3. 置顶不影响相邻关系（纯时间序 ≠ 列表 sticky 优先序） ----------

#[tokio::test(flavor = "multi_thread")]
async fn sticky_does_not_affect_neighbors() {
    let tmp = tempfile::tempdir().unwrap();
    let base = spawn_server(tmp.path().join("config.toml").to_str().unwrap()).await;
    let c = reqwest::Client::new();
    let token = setup_installed(&c, &base, tmp.path()).await;

    let list = get_json(&c, &format!("{base}/api/posts")).await;
    let [newest, middle, oldest] = samples(&list);

    // 置顶最老一篇 → 列表序变为 最老, 最新, 中间（sticky 优先）
    let oldest_id = list["items"][2]["id"].as_i64().unwrap();
    let (st, _) = admin_json(
        &c,
        "PATCH",
        &format!("{base}/api/admin/posts/{oldest_id}/sticky"),
        &token,
        Some(json!({"is_sticky": true})),
    )
    .await;
    assert_eq!(st, 200);
    let v = get_json(&c, &format!("{base}/api/posts")).await;
    assert_eq!(v["items"][0]["slug"], oldest.1, "置顶文章应排列表最前");

    // 相邻关系完全按时间序：
    // 最新一篇 prev=中间、next=null（列表序下它的前一条是置顶的最老，不能取）
    let d = get_json(&c, &format!("{base}/api/posts/{}", newest.1)).await;
    assert_eq!(nav_of(&d, "prev_post"), Some(middle.clone()));
    assert!(d["next_post"].is_null());
    // 置顶的最老一篇：next=中间（时间上的下一篇），而不是列表序的「最新」
    let d = get_json(&c, &format!("{base}/api/posts/{}", oldest.1)).await;
    assert!(d["prev_post"].is_null());
    assert_eq!(
        nav_of(&d, "next_post"),
        Some(middle.clone()),
        "置顶不改变相邻关系：next 应为时间上更晚的中间一篇"
    );
    // 中间一篇两侧不变
    let d = get_json(&c, &format!("{base}/api/posts/{}", middle.1)).await;
    assert_eq!(nav_of(&d, "prev_post"), Some(oldest.clone()));
    assert_eq!(nav_of(&d, "next_post"), Some(newest.clone()));
}

// ---------- 4. 同秒发布按 id 稳定 tiebreak（published_at DESC, id DESC） ----------

#[tokio::test(flavor = "multi_thread")]
async fn same_timestamp_tiebreak_by_id() {
    let tmp = tempfile::tempdir().unwrap();
    let cfg_path = tmp.path().join("config.toml");
    let base = spawn_server(cfg_path.to_str().unwrap()).await;
    let c = reqwest::Client::new();
    let token = setup_installed(&c, &base, tmp.path()).await;
    let db_url = sqlite_url_from_config(cfg_path.to_str().unwrap());

    // 先后创建 A、B（B 的 id 更大），再把 published_at 固定为同一时刻
    let mut ids = Vec::new();
    for (title, slug) in [("并列 A", "tie-a"), ("并列 B", "tie-b")] {
        let (st, v) = admin_json(
            &c,
            "POST",
            &format!("{base}/api/admin/posts"),
            &token,
            Some(json!({"title": title, "slug": slug, "content_md": "x", "status": "published"})),
        )
        .await;
        assert_eq!(st, 201);
        ids.push(v["id"].as_i64().unwrap());
    }
    let (a_id, b_id) = (ids[0], ids[1]);
    assert!(a_id < b_id);
    for id in [a_id, b_id] {
        db_set_published_at(&db_url, id, "2026-09-01T00:00:00Z").await;
    }

    let list = get_json(&c, &format!("{base}/api/posts?per_page=100")).await;
    let items = list["items"].as_array().unwrap();
    assert_eq!(items.len(), 5, "3 篇示例 + 并列 A/B");
    let find = |title: &str| {
        let it = items
            .iter()
            .find(|i| i["title"] == title)
            .unwrap_or_else(|| panic!("列表应含《{title}》"));
        (
            it["title"].as_str().unwrap().to_string(),
            it["slug"].as_str().unwrap().to_string(),
        )
    };
    let newest = find("欢迎使用 reedblog");
    let middle = find("用 Axum 和 SQLx 搭建轻量博客后端");

    let a = ("并列 A".to_string(), "tie-a".to_string());
    let b = ("并列 B".to_string(), "tie-b".to_string());

    // 相邻关系按 published_at DESC, id DESC：同秒组内 id 大者（B）在前 = 时间序更晚。
    // B：prev=A、next=样本最新（09-20）
    let d = get_json(&c, &format!("{base}/api/posts/tie-b")).await;
    assert_eq!(nav_of(&d, "prev_post"), Some(a.clone()));
    assert_eq!(nav_of(&d, "next_post"), Some(newest.clone()));
    // A（同秒组内 id 小者 = 更早）：prev=样本中间（08-15）、next=B
    let d = get_json(&c, &format!("{base}/api/posts/tie-a")).await;
    assert_eq!(nav_of(&d, "prev_post"), Some(middle.clone()));
    assert_eq!(nav_of(&d, "next_post"), Some(b.clone()));
    // 样本最新一篇的 prev 应为 B（同秒组里 id 更大者在前）
    let d = get_json(&c, &format!("{base}/api/posts/{}", newest.1)).await;
    assert_eq!(nav_of(&d, "prev_post"), Some(b.clone()));
}
