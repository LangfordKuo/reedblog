//! 文章修订历史集成测试（api-contract.md「文章修订历史」条款，2026-10-04 新增）：
//! - 新建文章即有 1 条初始修订；改 2 次（一次只改标题、一次只改正文）后共 3 条且按时间/id 倒序
//! - 内容未变化的保存不新增（含只改 status/tags 的 PUT）；PATCH sticky 不新增；
//!   仅改 excerpt 也算内容变化
//! - 列表摘要不含 content_md/excerpt（只给 content_chars）；单条 GET 返回完整正文
//! - restore 把该版本写回文章（含前台立即生效）且修订数 +1，status/置顶不被改动
//! - 超过 20 条裁剪最旧；删除文章连带清理修订
//! - 无鉴权 401；文章/修订不存在（含跨文章访问修订）404

use reedblog_backend::state::AppState;
use serde_json::{json, Value};
use sqlx::Row;
use std::path::Path;

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
            "site": {"title": "修订历史测试博客"}
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
    let mut req = c.request(method.parse().unwrap(), url);
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

/// GET 修订列表 → (HTTP 状态, JSON)
async fn list_revisions(
    c: &reqwest::Client,
    base: &str,
    token: Option<&str>,
    post_id: i64,
) -> (u16, Value) {
    json_req(
        c,
        "GET",
        &format!("{base}/api/admin/posts/{post_id}/revisions"),
        token,
        None,
    )
    .await
}

/// 列表里的标题序列（已按 API 顺序）
fn titles(list: &Value) -> Vec<String> {
    list.as_array()
        .unwrap()
        .iter()
        .map(|i| i["title"].as_str().unwrap().to_string())
        .collect()
}

/// 安装写入的 config.toml → SQLite 连接 URL（复用 Config::db_url 的路径编码逻辑）
fn sqlite_url_from_config(config_path: &str) -> String {
    reedblog_backend::config::Config::load(Path::new(config_path))
        .expect("config.toml 应已由安装流程写入")
        .db_url()
        .expect("db_type 应为 sqlite")
}

/// 直连 SQLite 数某文章的修订行数（验证删除连带清理）
async fn db_count_revisions(url: &str, post_id: i64) -> i64 {
    sqlx::any::install_default_drivers();
    let pool = sqlx::AnyPool::connect(url).await.unwrap();
    let count: i64 = sqlx::query("SELECT COUNT(*) FROM post_revisions WHERE post_id = ?")
        .bind(post_id)
        .fetch_one(&pool)
        .await
        .unwrap()
        .get(0);
    pool.close().await;
    count
}

// ---------- 1. 新建即有 1 条初始修订 + 形状 + 401/404 ----------

#[tokio::test(flavor = "multi_thread")]
async fn create_post_has_initial_revision_shape_auth_and_404() {
    let tmp = tempfile::tempdir().unwrap();
    let config_path = tmp.path().join("config.toml");
    let base = spawn_server(config_path.to_str().unwrap()).await;
    let c = reqwest::Client::new();
    let token = setup_installed(&c, &base, tmp.path()).await;

    let content = "你好world\n第二行";
    let post = create_post(
        &c,
        &base,
        &token,
        json!({"title": "初始标题", "content_md": content, "excerpt": "手动摘要", "status": "draft"}),
    )
    .await;
    let id = post["id"].as_i64().unwrap();

    // 列表：恰好 1 条初始修订；摘要形状只含 id/post_id/title/content_chars/created_at
    let (status, list) = list_revisions(&c, &base, Some(&token), id).await;
    assert_eq!(status, 200, "列表应 200: {list}");
    let arr = list.as_array().unwrap();
    assert_eq!(arr.len(), 1, "新建文章应有且仅有 1 条初始修订: {list}");
    let item = &arr[0];
    assert_eq!(item["post_id"].as_i64().unwrap(), id);
    assert_eq!(item["title"], "初始标题");
    assert_eq!(
        item["content_chars"].as_i64().unwrap(),
        content.chars().count() as i64
    );
    assert!(item.get("content_md").is_none(), "摘要不得返回 content_md");
    assert!(item.get("excerpt").is_none(), "摘要不得返回 excerpt");
    assert!(
        item["created_at"].as_str().unwrap().len() >= 20,
        "created_at 应为 RFC3339"
    );

    // 单条：完整正文 + 摘要
    let rev_id = item["id"].as_i64().unwrap();
    let (status, detail) = json_req(
        &c,
        "GET",
        &format!("{base}/api/admin/posts/{id}/revisions/{rev_id}"),
        Some(&token),
        None,
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(detail["content_md"], content);
    assert_eq!(detail["excerpt"], "手动摘要");
    assert_eq!(detail["created_at"], item["created_at"]);

    // 无鉴权 → 401（列表 / 单条 / 恢复）
    let (s, v) = list_revisions(&c, &base, None, id).await;
    assert_eq!(s, 401, "列表未登录应 401");
    assert_eq!(v["error"]["code"], "unauthorized");
    let (s, v) = json_req(
        &c,
        "GET",
        &format!("{base}/api/admin/posts/{id}/revisions/{rev_id}"),
        None,
        None,
    )
    .await;
    assert_eq!(s, 401);
    assert_eq!(v["error"]["code"], "unauthorized");
    let (s, v) = json_req(
        &c,
        "POST",
        &format!("{base}/api/admin/posts/{id}/revisions/{rev_id}/restore"),
        None,
        None,
    )
    .await;
    assert_eq!(s, 401);
    assert_eq!(v["error"]["code"], "unauthorized");

    // 文章不存在 → 404（列表 / 单条 / 恢复）
    let (s, v) = list_revisions(&c, &base, Some(&token), 999_999).await;
    assert_eq!(s, 404, "不存在文章的修订列表应 404");
    assert_eq!(v["error"]["code"], "not_found");
    let (s, _) = json_req(
        &c,
        "GET",
        &format!("{base}/api/admin/posts/999999/revisions/{rev_id}"),
        Some(&token),
        None,
    )
    .await;
    assert_eq!(s, 404);
    let (s, _) = json_req(
        &c,
        "POST",
        &format!("{base}/api/admin/posts/999999/revisions/{rev_id}/restore"),
        Some(&token),
        None,
    )
    .await;
    assert_eq!(s, 404);

    // 文章存在、修订不存在 → 404（单条 / 恢复）
    let (s, _) = json_req(
        &c,
        "GET",
        &format!("{base}/api/admin/posts/{id}/revisions/999999"),
        Some(&token),
        None,
    )
    .await;
    assert_eq!(s, 404, "不存在修订应 404");
    let (s, _) = json_req(
        &c,
        "POST",
        &format!("{base}/api/admin/posts/{id}/revisions/999999/restore"),
        Some(&token),
        None,
    )
    .await;
    assert_eq!(s, 404, "恢复不存在修订应 404");
}

// ---------- 2. 内容变化才新增；未变化保存 / sticky / tags 不新增 ----------

#[tokio::test(flavor = "multi_thread")]
async fn revisions_only_added_when_content_changes() {
    let tmp = tempfile::tempdir().unwrap();
    let config_path = tmp.path().join("config.toml");
    let base = spawn_server(config_path.to_str().unwrap()).await;
    let c = reqwest::Client::new();
    let token = setup_installed(&c, &base, tmp.path()).await;

    let post = create_post(
        &c,
        &base,
        &token,
        json!({"title": "T0", "content_md": "第一版正文", "excerpt": "摘要0", "status": "draft"}),
    )
    .await;
    let id = post["id"].as_i64().unwrap();
    let base_url = format!("{base}/api/admin/posts/{id}");

    // 第 1 次修改：只改标题
    let (s, _) = json_req(&c, "PUT", &base_url, Some(&token), Some(json!({"title": "T1"}))).await;
    assert_eq!(s, 200);
    let (_, list) = list_revisions(&c, &base, Some(&token), id).await;
    assert_eq!(list.as_array().unwrap().len(), 2);
    assert_eq!(titles(&list), vec!["T1", "T0"]);

    // 第 2 次修改：只改正文（标题保持 T1）→ 共 3 条，倒序（秒级时间戳下 id DESC 兜底）
    let (s, _) = json_req(
        &c,
        "PUT",
        &base_url,
        Some(&token),
        Some(json!({"content_md": "第二版正文更长一些"})),
    )
    .await;
    assert_eq!(s, 200);
    let (_, list) = list_revisions(&c, &base, Some(&token), id).await;
    assert_eq!(list.as_array().unwrap().len(), 3, "改 2 次后共 3 条: {list}");
    assert_eq!(titles(&list), vec!["T1", "T1", "T0"], "应按时间/id 倒序");
    let chars: Vec<i64> = list
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["content_chars"].as_i64().unwrap())
        .collect();
    assert_eq!(chars, vec![9, 5, 5], "最后一次改正文的字符数应最新");

    // 内容未变化（title/content_md 与库中一致；excerpt 缺省保持）→ 不新增
    let (s, _) = json_req(
        &c,
        "PUT",
        &base_url,
        Some(&token),
        Some(json!({"title": "T1", "content_md": "第二版正文更长一些"})),
    )
    .await;
    assert_eq!(s, 200);
    // 只改 status（发布）→ 不新增
    let (s, _) = json_req(&c, "PUT", &base_url, Some(&token), Some(json!({"status": "published"}))).await;
    assert_eq!(s, 200);
    // 行内 PATCH 置顶 → 不新增
    let (s, _) = json_req(
        &c,
        "PATCH",
        &format!("{base}/api/admin/posts/{id}/sticky"),
        Some(&token),
        Some(json!({"is_sticky": true})),
    )
    .await;
    assert_eq!(s, 200);
    // 只改标签 → 不新增
    let (_, tag) = json_req(
        &c,
        "POST",
        &format!("{base}/api/admin/tags"),
        Some(&token),
        Some(json!({"name": "修订测试标签"})),
    )
    .await;
    let tag_id = tag["id"].as_i64().unwrap();
    let (s, _) = json_req(
        &c,
        "PUT",
        &base_url,
        Some(&token),
        Some(json!({"tag_ids": [tag_id]})),
    )
    .await;
    assert_eq!(s, 200);

    let (_, list) = list_revisions(&c, &base, Some(&token), id).await;
    assert_eq!(
        list.as_array().unwrap().len(),
        3,
        "未变化保存 / status / sticky / tags 均不得新增修订: {list}"
    );

    // 仅改 excerpt 也算内容变化 → +1
    let (s, _) = json_req(
        &c,
        "PUT",
        &base_url,
        Some(&token),
        Some(json!({"excerpt": "摘要1"})),
    )
    .await;
    assert_eq!(s, 200);
    let (_, list) = list_revisions(&c, &base, Some(&token), id).await;
    assert_eq!(list.as_array().unwrap().len(), 4, "仅改 excerpt 也应新增");
}

// ---------- 3. restore：写回 + 留痕 +1 + 前台生效 + 状态/置顶不变 ----------

#[tokio::test(flavor = "multi_thread")]
async fn restore_writes_back_content_and_adds_revision() {
    let tmp = tempfile::tempdir().unwrap();
    let config_path = tmp.path().join("config.toml");
    let base = spawn_server(config_path.to_str().unwrap()).await;
    let c = reqwest::Client::new();
    let token = setup_installed(&c, &base, tmp.path()).await;

    // 草稿 A → 发布并改正文为 B → 置顶
    let post = create_post(
        &c,
        &base,
        &token,
        json!({"title": "标题A", "content_md": "内容A", "excerpt": "摘要A", "status": "draft"}),
    )
    .await;
    let id = post["id"].as_i64().unwrap();
    let slug = post["slug"].as_str().unwrap().to_string();
    let base_url = format!("{base}/api/admin/posts/{id}");

    let (s, _) = json_req(
        &c,
        "PUT",
        &base_url,
        Some(&token),
        Some(json!({"content_md": "内容B", "status": "published"})),
    )
    .await;
    assert_eq!(s, 200);
    let (s, _) = json_req(
        &c,
        "PATCH",
        &format!("{base}/api/admin/posts/{id}/sticky"),
        Some(&token),
        Some(json!({"is_sticky": true})),
    )
    .await;
    assert_eq!(s, 200);

    // 修订序：[B(id2), A(id1)]
    let (_, list) = list_revisions(&c, &base, Some(&token), id).await;
    assert_eq!(list.as_array().unwrap().len(), 2);
    let rev_a = list.as_array().unwrap()[1]["id"].as_i64().unwrap();

    // 恢复 A → PostAdmin：内容回到 A，status/置顶不受影响
    let (s, restored) = json_req(
        &c,
        "POST",
        &format!("{base}/api/admin/posts/{id}/revisions/{rev_a}/restore"),
        Some(&token),
        None,
    )
    .await;
    assert_eq!(s, 200, "恢复应 200: {restored}");
    assert_eq!(restored["title"], "标题A");
    assert_eq!(restored["content_md"], "内容A");
    assert_eq!(restored["excerpt"], "摘要A");
    assert_eq!(restored["status"], "published", "恢复不得改动状态");
    assert_eq!(restored["is_sticky"], true, "恢复不得改动置顶");
    assert_ne!(restored["updated_at"], Value::Null);

    // 前台立即生效（公开详情返回恢复后的正文）
    let (s, public) = json_req(
        &c,
        "GET",
        &format!("{base}/api/posts/{slug}"),
        None,
        None,
    )
    .await;
    assert_eq!(s, 200);
    assert_eq!(public["content_md"], "内容A");

    // 回滚本身留痕：修订数 +1，最新一条即恢复后的内容
    let (_, list) = list_revisions(&c, &base, Some(&token), id).await;
    let arr = list.as_array().unwrap();
    assert_eq!(arr.len(), 3, "恢复后修订数应 +1: {list}");
    assert_eq!(arr[0]["title"], "标题A");
    assert_eq!(
        arr[0]["content_chars"].as_i64().unwrap(),
        "内容A".chars().count() as i64
    );
    let new_rev = arr[0]["id"].as_i64().unwrap();
    let (s, detail) = json_req(
        &c,
        "GET",
        &format!("{base}/api/admin/posts/{id}/revisions/{new_rev}"),
        Some(&token),
        None,
    )
    .await;
    assert_eq!(s, 200);
    assert_eq!(detail["content_md"], "内容A");

    // 跨文章访问修订 → 404（且不影响另一篇文章内容）
    let other = create_post(
        &c,
        &base,
        &token,
        json!({"title": "其他文章", "content_md": "其他内容", "status": "draft"}),
    )
    .await;
    let other_id = other["id"].as_i64().unwrap();
    let (s, _) = json_req(
        &c,
        "GET",
        &format!("{base}/api/admin/posts/{other_id}/revisions/{rev_a}"),
        Some(&token),
        None,
    )
    .await;
    assert_eq!(s, 404, "修订不属于该文章应 404");
    let (s, _) = json_req(
        &c,
        "POST",
        &format!("{base}/api/admin/posts/{other_id}/revisions/{rev_a}/restore"),
        Some(&token),
        None,
    )
    .await;
    assert_eq!(s, 404);
    let (_, other_after) = json_req(
        &c,
        "GET",
        &format!("{base}/api/admin/posts/{other_id}"),
        Some(&token),
        None,
    )
    .await;
    assert_eq!(other_after["content_md"], "其他内容", "404 的恢复不得改动文章");
}

// ---------- 4. 保留上限：超过 20 条裁剪最旧 ----------

#[tokio::test(flavor = "multi_thread")]
async fn retention_keeps_latest_20() {
    let tmp = tempfile::tempdir().unwrap();
    let config_path = tmp.path().join("config.toml");
    let base = spawn_server(config_path.to_str().unwrap()).await;
    let c = reqwest::Client::new();
    let token = setup_installed(&c, &base, tmp.path()).await;

    let post = create_post(
        &c,
        &base,
        &token,
        json!({"title": "标题0", "content_md": "正文0", "status": "draft"}),
    )
    .await;
    let id = post["id"].as_i64().unwrap();
    let base_url = format!("{base}/api/admin/posts/{id}");

    // 初始 1 条 + 25 次修改 = 26 次插入 → 只保留最近 20 条（最旧被裁）
    for i in 1..=25 {
        let (s, _) = json_req(
            &c,
            "PUT",
            &base_url,
            Some(&token),
            Some(json!({"title": format!("标题{i}"), "content_md": format!("正文{i}")})),
        )
        .await;
        assert_eq!(s, 200, "第 {i} 次修改失败");
    }

    let (_, list) = list_revisions(&c, &base, Some(&token), id).await;
    let arr = list.as_array().unwrap();
    assert_eq!(arr.len(), 20, "每篇最多保留 20 条");
    let ts = titles(&list);
    assert_eq!(ts.first().unwrap(), "标题25", "最新一条应是最后一次保存");
    assert_eq!(ts.last().unwrap(), "标题6", "最旧的 6 条（初始 + 1~5）应被裁掉");
    assert_eq!(
        arr[19]["content_chars"].as_i64().unwrap(),
        "正文6".chars().count() as i64
    );
}

// ---------- 5. 删除文章连带清理修订 ----------

#[tokio::test(flavor = "multi_thread")]
async fn deleting_post_cascades_revisions() {
    let tmp = tempfile::tempdir().unwrap();
    let config_path = tmp.path().join("config.toml");
    let base = spawn_server(config_path.to_str().unwrap()).await;
    let c = reqwest::Client::new();
    let token = setup_installed(&c, &base, tmp.path()).await;

    let post = create_post(
        &c,
        &base,
        &token,
        json!({"title": "待删除", "content_md": "正文", "status": "draft"}),
    )
    .await;
    let id = post["id"].as_i64().unwrap();

    let url = sqlite_url_from_config(config_path.to_str().unwrap());
    assert_eq!(db_count_revisions(&url, id).await, 1, "删除前应有初始修订");

    let (s, _) = json_req(
        &c,
        "DELETE",
        &format!("{base}/api/admin/posts/{id}"),
        Some(&token),
        None,
    )
    .await;
    assert_eq!(s, 204, "删除文章应 204");

    assert_eq!(db_count_revisions(&url, id).await, 0, "删除文章应连带清理修订");
    let (s, _) = list_revisions(&c, &base, Some(&token), id).await;
    assert_eq!(s, 404, "文章不存在后修订列表应 404");
}
