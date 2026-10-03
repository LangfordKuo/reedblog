//! 全文搜索集成测试（契约「全文搜索」条款，2026-10-03 新增）：
//! - title 命中 / content_md 命中 / 都不命中；draft 永不出现
//! - 多词条 AND 语义；CJK 词条；英文大小写不敏感
//! - LIKE 通配符转义（搜 `100%`、`%`、`_` 不会变成全匹配）
//! - q 缺失/为空 → 400 validation_error
//! - snippet：窗口截取与 `…`、无 HTML/Markdown 符号、纯文本无命中回退 excerpt
//! - 分页：per_page=1 时 total 正确、翻页不重不漏

use reedblog_backend::state::AppState;
use serde_json::{json, Value};
use std::collections::HashSet;
use std::path::Path;

/// 在 127.0.0.1 随机端口起真实服务（与 integration.rs 同款隔离：插件/主题目录指向 tempdir）
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

/// 建一篇文章（管理接口），返回 id
async fn create_post(c: &reqwest::Client, base: &str, token: &str, body: Value) -> i64 {
    let r = c
        .post(format!("{base}/api/admin/posts"))
        .bearer_auth(token)
        .json(&body)
        .send()
        .await
        .unwrap();
    let status = r.status();
    let text = r.text().await.unwrap();
    assert_eq!(status, 201, "创建文章失败: {text}");
    serde_json::from_str::<Value>(&text).unwrap()["id"]
        .as_i64()
        .unwrap()
}

/// 夹具文章 id 集合
struct Fixtures {
    /// title 命中（"Rust 入门指南"，excerpt 手填"入门要点汇总"，正文无"入门"）
    rust_intro: i64,
    /// 正文命中（title "周报"，content 含 Rust/所有权）
    weekly: i64,
    /// 常规词都不命中，但正文含 "100"（无 %），用于通配符转义对照
    coffee: i64,
    /// draft：搜任何词都不得出现
    draft: i64,
    /// 正文含字面 "100%"（通配符转义目标）
    sale: i64,
    /// 长文：needle 前后各 80 字，用于 snippet 窗口截取
    long_post: i64,
    /// 安装时自动注入的示例文章 id（契约「安装向导」条款）；
    /// 示例内容可能同样命中搜索词，夹具断言一律先排除这批 id
    samples: HashSet<i64>,
}

/// 安装 + 登录 + 建 6 篇夹具文章（5 published + 1 draft）
async fn setup(dir: &Path) -> (reqwest::Client, String, String, Fixtures) {
    let base = spawn_server(dir.join("config.toml").to_str().unwrap()).await;
    let c = reqwest::Client::new();
    let r = c
        .post(format!("{base}/api/install"))
        .json(&json!({
            "db_type": "sqlite",
            "sqlite_path": dir.join("reedblog.db").to_str().unwrap(),
            "admin": {"username": "admin", "password": "secret123"},
            "site": {"title": "搜索测试站"}
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 201, "install 失败: {}", r.text().await.unwrap());
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

    // 安装自动注入了示例文章：先记下它们的 id，夹具断言需排除
    let v = c
        .get(format!("{base}/api/posts?per_page=100"))
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    let samples: HashSet<i64> = v["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["id"].as_i64().unwrap())
        .collect();
    assert!(!samples.is_empty(), "安装后应有示例文章");

    // needle 前后各 80 个"字"，且用 Markdown 强调/标题包裹（snippet 必须剥净符号）
    let long_content = format!(
        "## 小节\n\n**{}针点{}**尾声",
        "字".repeat(80),
        "字".repeat(80)
    );

    let rust_intro = create_post(
        &c,
        &base,
        &token,
        json!({
            "title": "Rust 入门指南", "content_md": "Axum 是一个不错的 web framework，基于 Tokio 构建",
            "excerpt": "入门要点汇总", "status": "published"
        }),
    )
    .await;
    let weekly = create_post(
        &c,
        &base,
        &token,
        json!({
            "title": "周报", "content_md": "这周学习了 Rust 的所有权与借用检查",
            "status": "published"
        }),
    )
    .await;
    let coffee = create_post(
        &c,
        &base,
        &token,
        json!({
            "title": "咖啡冲煮", "content_md": "手冲咖啡有一百个技巧，水温最重要",
            "status": "published"
        }),
    )
    .await;
    let draft = create_post(
        &c,
        &base,
        &token,
        json!({
            "title": "未发布笔记", "content_md": "Rust 异步编程", "status": "draft"
        }),
    )
    .await;
    let sale = create_post(
        &c,
        &base,
        &token,
        json!({
            "title": "限时优惠", "content_md": "全场商品 100% off，仅限前十名",
            "status": "published"
        }),
    )
    .await;
    let long_post = create_post(
        &c,
        &base,
        &token,
        json!({
            "title": "长文", "content_md": long_content, "status": "published"
        }),
    )
    .await;

    (
        c,
        base,
        token,
        Fixtures {
            rust_intro,
            weekly,
            coffee,
            draft,
            sale,
            long_post,
            samples,
        },
    )
}

/// GET /api/search<query> → 200 JSON
async fn search(c: &reqwest::Client, base: &str, query: &str) -> Value {
    let r = c
        .get(format!("{base}/api/search{query}"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200, "GET /api/search{query}");
    r.json().await.unwrap()
}

/// 结果 items 的 id 集合
fn ids(v: &Value) -> HashSet<i64> {
    v["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["id"].as_i64().unwrap())
        .collect()
}

/// 结果 items 的 id 集合，排除安装注入的示例文章（命中语义断言只针对夹具文章；
/// 示例内容也可能含搜索词，未转义通配符时同样会全体命中，故排除后断言仍是完整证明）
fn fx_ids(v: &Value, fx: &Fixtures) -> HashSet<i64> {
    ids(v).difference(&fx.samples).copied().collect()
}

/// 在结果 items 中按 id 定位单条（不存在则 panic）
fn item_by_id<'a>(v: &'a Value, id: i64) -> &'a Value {
    v["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["id"].as_i64().unwrap() == id)
        .unwrap_or_else(|| panic!("搜索结果缺少文章 {id}: {v}"))
}

// ---------- 1. 命中范围 / AND 语义 / draft 排除 / CJK / 大小写 ----------

#[tokio::test(flavor = "multi_thread")]
async fn search_matching_semantics() {
    let tmp = tempfile::tempdir().unwrap();
    let (c, base, _token, fx) = setup(tmp.path()).await;

    // title 命中 + 正文命中都返回；draft 永不出现（示例文章可能同样命中，断言用 fx_ids 排除）
    let v = search(&c, &base, "?q=Rust").await;
    assert_eq!(fx_ids(&v, &fx), HashSet::from([fx.rust_intro, fx.weekly]));
    assert!(!ids(&v).contains(&fx.draft), "draft 不得出现在搜索结果");

    // draft 独有词 → 夹具零命中（进一步验证 draft 被排除）
    let v = search(&c, &base, "?q=异步").await;
    assert!(fx_ids(&v, &fx).is_empty());
    assert!(!ids(&v).contains(&fx.draft), "draft 不得出现在搜索结果");

    // 英文大小写不敏感
    assert_eq!(
        fx_ids(&search(&c, &base, "?q=rust").await, &fx),
        HashSet::from([fx.rust_intro, fx.weekly])
    );
    assert_eq!(
        fx_ids(&search(&c, &base, "?q=AxUm").await, &fx),
        HashSet::from([fx.rust_intro])
    );

    // CJK 词条：title 命中 / 正文命中
    assert_eq!(
        fx_ids(&search(&c, &base, "?q=入门").await, &fx),
        HashSet::from([fx.rust_intro])
    );
    assert_eq!(
        fx_ids(&search(&c, &base, "?q=所有权").await, &fx),
        HashSet::from([fx.weekly])
    );
    assert_eq!(
        fx_ids(&search(&c, &base, "?q=手冲咖啡").await, &fx),
        HashSet::from([fx.coffee])
    );

    // 多词条 AND：两词都有 → 命中；只有一词 → 夹具不命中
    assert_eq!(
        fx_ids(&search(&c, &base, "?q=Rust%20所有权").await, &fx),
        HashSet::from([fx.weekly])
    );
    let v = search(&c, &base, "?q=Rust%20咖啡").await;
    assert!(fx_ids(&v, &fx).is_empty());

    // 响应形状：Page 壳 + SearchResult（PostPublic 全字段 + snippet）
    let item = &search(&c, &base, "?q=Rust").await["items"][0];
    for key in [
        "id",
        "title",
        "slug",
        "excerpt",
        "category",
        "tags",
        "published_at",
        "comment_count",
        "snippet",
    ] {
        assert!(item.get(key).is_some(), "SearchResult 缺少字段 {key}");
    }
}

// ---------- 2. LIKE 通配符转义 ----------

#[tokio::test(flavor = "multi_thread")]
async fn search_escapes_like_wildcards() {
    let tmp = tempfile::tempdir().unwrap();
    let (c, base, _token, fx) = setup(tmp.path()).await;

    // 搜 "100%"：夹具中只命中含字面 100% 的文章；不含 % 的（"一百个技巧"无 100）不误中。
    // 若未转义，%100%% 等价"含 100 即命中"，本夹具中恰只有 sale 含 "100"，
    // 因此再用单 "%" 搜索证明转义：未转义时 %%% 会全匹配所有 published（含全部夹具）。
    assert_eq!(
        fx_ids(&search(&c, &base, "?q=100%25").await, &fx),
        HashSet::from([fx.sale])
    );
    let v = search(&c, &base, "?q=%25").await;
    assert_eq!(
        fx_ids(&v, &fx),
        HashSet::from([fx.sale]),
        "搜 '%' 夹具中只应命中含字面 % 的文章"
    );

    // 下划线同理：没有任何夹具文章含 "_" → 夹具零命中（未转义时 %_% 匹配一切非空正文）
    let v = search(&c, &base, "?q=_").await;
    assert!(fx_ids(&v, &fx).is_empty());

    // 转义不影响正常词条把 % 当普通字符组合搜索
    assert_eq!(
        fx_ids(&search(&c, &base, "?q=100%25%20off").await, &fx),
        HashSet::from([fx.sale])
    );
}

// ---------- 3. q 校验 ----------

#[tokio::test(flavor = "multi_thread")]
async fn search_requires_non_empty_q() {
    let tmp = tempfile::tempdir().unwrap();
    let (c, base, _token, _fx) = setup(tmp.path()).await;

    for query in ["", "?q=", "?q=%20%20"] {
        let r = c
            .get(format!("{base}/api/search{query}"))
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), 400, "GET /api/search{query} 应 400");
        assert_eq!(err_code(r).await, "validation_error", "{query}");
    }
}

// ---------- 4. snippet：窗口截取 / 无 markup / 回退 excerpt ----------

#[tokio::test(flavor = "multi_thread")]
async fn search_snippet_window_and_fallback() {
    let tmp = tempfile::tempdir().unwrap();
    let (c, base, _token, fx) = setup(tmp.path()).await;

    // 长文命中：窗口 = 前 ≤40 + 词条 + 后 ≤60，两端 `…`，且无 Markdown 符号
    // （示例文章可能同样命中，snippet 断言按夹具 id 定位条目）
    let v = search(&c, &base, "?q=针点").await;
    assert_eq!(fx_ids(&v, &fx), HashSet::from([fx.long_post]));
    let snippet = item_by_id(&v, fx.long_post)["snippet"].as_str().unwrap();
    assert!(snippet.contains("针点"), "{snippet}");
    assert!(snippet.starts_with('…'), "窗口前端应有省略号: {snippet}");
    assert!(snippet.ends_with('…'), "窗口后端应有省略号: {snippet}");
    assert!(
        snippet.chars().count() <= 1 + 40 + 2 + 60 + 1,
        "窗口超长: {}",
        snippet.chars().count()
    );
    for sym in ['#', '*', '`', '|', '[', ']'] {
        assert!(
            !snippet.contains(sym),
            "snippet 不应含 markup 符号 {sym}: {snippet}"
        );
    }

    // 纯文本无命中（"入门"只在 title/excerpt）→ 回退 excerpt
    let v = search(&c, &base, "?q=入门").await;
    assert_eq!(fx_ids(&v, &fx), HashSet::from([fx.rust_intro]));
    let item = item_by_id(&v, fx.rust_intro);
    assert_eq!(item["snippet"], "入门要点汇总");
    assert_eq!(item["snippet"], item["excerpt"]);

    // 正文命中（无回退）：snippet 含命中词上下文、短文本无省略号
    let v = search(&c, &base, "?q=Tokio").await;
    let snippet = item_by_id(&v, fx.rust_intro)["snippet"].as_str().unwrap();
    assert!(snippet.contains("Tokio"), "{snippet}");
    assert!(!snippet.contains('…'), "短文本不应加省略号: {snippet}");
}

// ---------- 5. 分页 ----------

#[tokio::test(flavor = "multi_thread")]
async fn search_pagination() {
    let tmp = tempfile::tempdir().unwrap();
    let (c, base, _token, fx) = setup(tmp.path()).await;

    // per_page=1：total 为全量命中数（夹具 2 篇 + 可能命中的示例文章），items 只有 1 条
    let p1 = search(&c, &base, "?q=Rust&per_page=1").await;
    let total = p1["total"].as_i64().unwrap();
    assert!(total >= 2, "至少命中两篇夹具文章: {p1}");
    assert_eq!(p1["page"], 1);
    assert_eq!(p1["per_page"], 1);
    assert_eq!(p1["items"].as_array().unwrap().len(), 1);

    // 逐页翻完：不重不漏，两篇夹具文章都出现
    let mut all = ids(&p1);
    for page in 2..=total {
        let p = search(&c, &base, &format!("?q=Rust&per_page=1&page={page}")).await;
        assert_eq!(p["total"], total);
        assert_eq!(p["page"], page);
        assert_eq!(p["items"].as_array().unwrap().len(), 1);
        all.extend(ids(&p));
    }
    assert_eq!(all.len(), total as usize, "翻页不重不漏");
    assert!(all.contains(&fx.rust_intro) && all.contains(&fx.weekly));

    // 超出末页 → 空 items，total 不变
    let p9 = search(&c, &base, &format!("?q=Rust&per_page=1&page={}", total + 5)).await;
    assert_eq!(p9["total"], total);
    assert!(p9["items"].as_array().unwrap().is_empty());
}
