//! 嵌套评论（楼中楼回复）集成测试（契约「评论回复」条款，2026-10-03 新增）：
//! - 带 parent_id 创建回复；回复的回复两级归一化（parent_id=顶级祖先 + reply_to_id 保留）
//! - 父不存在 / 跨目标（跨文章、文章↔留言板）/ 父被隐藏 → 422 validation_error
//! - 公开列表平铺 + reply_to_name 正确（JOIN 被回复人作者名）
//! - 管理端 CommentAdmin 带 parent_id/reply_to_id/reply_to_name/reply_count
//! - 删顶级评论连带删子回复；删子回复只删自身
//! - hidden 顶级评论 → 前台整条线程不可见（含 comment_count），恢复后整体重现
//! - comment.before_create 钩子对回复生效（ctx.parent_id/reply_to_id 为归一化后的值）

use reedblog_backend::state::AppState;
use serde_json::{json, Value};
use std::io::Write;
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
    err_code_msg(r).await.0
}

/// 提取契约错误形状的 (code, message)
async fn err_code_msg(r: reqwest::Response) -> (String, String) {
    let v: Value = r.json().await.unwrap();
    (
        v["error"]["code"].as_str().unwrap_or_default().to_string(),
        v["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .to_string(),
    )
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
            "site": {"title": "回复测试站"}
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

/// 管理端发文（published），返回文章 id
async fn publish_post(c: &reqwest::Client, base: &str, token: &str, title: &str) -> i64 {
    let r = c
        .post(format!("{base}/api/admin/posts"))
        .bearer_auth(token)
        .json(&json!({"title": title, "content_md": "正文", "status": "published"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 201);
    r.json::<Value>().await.unwrap()["id"].as_i64().unwrap()
}

/// 公开端发评论/回复，返回 201 的 CommentPub
async fn post_comment(c: &reqwest::Client, base: &str, slug: &str, body: Value) -> Value {
    let r = c
        .post(format!("{base}/api/posts/{slug}/comments"))
        .json(&body)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 201, "发评论应 201: {body}");
    r.json().await.unwrap()
}

/// GET 公开评论列表 → 数组
async fn public_comments(c: &reqwest::Client, base: &str, slug: &str) -> Vec<Value> {
    let v = c
        .get(format!("{base}/api/posts/{slug}/comments"))
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    v.as_array().unwrap().clone()
}

// ---------- 1. 回复创建、两级归一化、reply_to_name、校验 422 ----------

#[tokio::test(flavor = "multi_thread")]
async fn reply_creation_normalization_and_validation() {
    let tmp = tempfile::tempdir().unwrap();
    let (c, base, token) = setup(tmp.path()).await;
    publish_post(&c, &base, &token, "Reply Target").await;
    let slug = "reply-target";

    // 顶级评论 A：不带 parent_id → 三个新字段全 null
    let a = post_comment(
        &c,
        &base,
        slug,
        json!({"author_name": "甲", "content": "顶级评论A"}),
    )
    .await;
    let a_id = a["id"].as_i64().unwrap();
    assert_eq!(a["parent_id"], Value::Null);
    assert_eq!(a["reply_to_id"], Value::Null);
    assert_eq!(a["reply_to_name"], Value::Null);

    // 回复 B（parent=A，A 是顶级）→ parent_id=A，reply_to_id/reply_to_name 为 null
    let b = post_comment(
        &c,
        &base,
        slug,
        json!({"author_name": "乙", "content": "回复A", "parent_id": a_id}),
    )
    .await;
    let b_id = b["id"].as_i64().unwrap();
    assert_eq!(b["parent_id"], a_id);
    assert_eq!(b["reply_to_id"], Value::Null);
    assert_eq!(b["reply_to_name"], Value::Null);

    // 回复的回复 C（parent=B，B 本身有 parent）→ 两级归一化：
    // parent_id 改写为顶级祖先 A，reply_to_id=B，reply_to_name=B 的作者「乙」
    let cc = post_comment(
        &c,
        &base,
        slug,
        json!({"author_name": "丙", "content": "回复B", "parent_id": b_id}),
    )
    .await;
    let c_id = cc["id"].as_i64().unwrap();
    assert_eq!(cc["parent_id"], a_id, "回复的回复应归一化到顶级祖先");
    assert_eq!(
        cc["reply_to_id"], b_id,
        "被回复的中间楼层保留在 reply_to_id"
    );
    assert_eq!(cc["reply_to_name"], "乙");

    // 更深层 D（parent=C）→ 仍归一化到 A，reply_to=C/「丙」
    let d = post_comment(
        &c,
        &base,
        slug,
        json!({"author_name": "丁", "content": "回复C", "parent_id": c_id}),
    )
    .await;
    assert_eq!(d["parent_id"], a_id);
    assert_eq!(d["reply_to_id"], c_id);
    assert_eq!(d["reply_to_name"], "丙");

    // 公开列表：仍是平铺数组、时间 ASC（父先于子），每项带三个新字段
    let list = public_comments(&c, &base, slug).await;
    assert_eq!(list.len(), 4);
    assert_eq!(list[0]["id"], a_id);
    assert_eq!(list[3]["id"], d["id"]);
    // reply_to_name 由 JOIN 取被回复人作者名
    assert_eq!(list[2]["reply_to_name"], "乙");
    assert_eq!(list[3]["reply_to_name"], "丙");
    assert_eq!(list[1]["parent_id"], a_id);
    assert_eq!(list[0]["parent_id"], Value::Null);

    // 校验：父评论不存在 → 422 validation_error（带明确 message）
    let r = c
        .post(format!("{base}/api/posts/{slug}/comments"))
        .json(&json!({"author_name": "x", "content": "y", "parent_id": 99999}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 422);
    assert_eq!(err_code(r).await, "validation_error");

    let r = c
        .post(format!("{base}/api/posts/{slug}/comments"))
        .json(&json!({"author_name": "x", "content": "y", "parent_id": 0}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 422);
    assert_eq!(err_code(r).await, "validation_error");

    // 校验：跨目标（父评论属于另一篇文章）→ 422
    publish_post(&c, &base, &token, "Other Post").await;
    let other_a = post_comment(
        &c,
        &base,
        "other-post",
        json!({"author_name": "戊", "content": "别的文章的评论"}),
    )
    .await;
    let r = c
        .post(format!("{base}/api/posts/{slug}/comments"))
        .json(&json!({"author_name": "x", "content": "y", "parent_id": other_a["id"]}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 422);
    let (code, msg) = err_code_msg(r).await;
    assert_eq!(code, "validation_error");
    assert!(msg.contains("目标"), "message 应明确说明跨目标: {msg}");

    // 校验：父评论被隐藏（status != approved）→ 422
    let r = c
        .put(format!("{base}/api/admin/comments/{b_id}"))
        .bearer_auth(&token)
        .json(&json!({"status": "hidden"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    let r = c
        .post(format!("{base}/api/posts/{slug}/comments"))
        .json(&json!({"author_name": "x", "content": "y", "parent_id": b_id}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 422);
    assert_eq!(err_code(r).await, "validation_error");
    // 恢复，避免影响后续断言
    let r = c
        .put(format!("{base}/api/admin/comments/{b_id}"))
        .bearer_auth(&token)
        .json(&json!({"status": "approved"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    // 恢复后仍可正常回复（父 approved）
    let e = post_comment(
        &c,
        &base,
        slug,
        json!({"author_name": "己", "content": "恢复后的回复", "parent_id": b_id}),
    )
    .await;
    assert_eq!(e["parent_id"], a_id);
    assert_eq!(e["reply_to_id"], b_id);
}

#[tokio::test(flavor = "multi_thread")]
async fn admin_list_fields_cascade_delete_and_hidden_thread() {
    let tmp = tempfile::tempdir().unwrap();
    let (c, base, token) = setup(tmp.path()).await;
    let post_id = publish_post(&c, &base, &token, "Admin Thread").await;
    let slug = "admin-thread";

    let a = post_comment(
        &c,
        &base,
        slug,
        json!({"author_name": "楼主", "content": "A"}),
    )
    .await;
    let a_id = a["id"].as_i64().unwrap();
    let b = post_comment(
        &c,
        &base,
        slug,
        json!({"author_name": "层主", "content": "B", "parent_id": a_id}),
    )
    .await;
    let b_id = b["id"].as_i64().unwrap();
    let cc = post_comment(
        &c,
        &base,
        slug,
        json!({"author_name": "层层主", "content": "C", "parent_id": b_id}),
    )
    .await;
    let c_id = cc["id"].as_i64().unwrap();

    // 管理端列表（post_id 过滤 = 目标文章）：字段与 reply_count 正确，排序仍 created_at DESC
    let v = c
        .get(format!("{base}/api/admin/comments?post_id={post_id}"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    assert_eq!(v["total"], 3);
    let items = v["items"].as_array().unwrap();
    assert_eq!(items[0]["id"], c_id, "created_at DESC：最新在前");
    // C：parent_id=A（归一化）、reply_to_id=B、reply_to_name=层主、reply_count=0
    assert_eq!(items[0]["parent_id"], a_id);
    assert_eq!(items[0]["reply_to_id"], b_id);
    assert_eq!(items[0]["reply_to_name"], "层主");
    assert_eq!(items[0]["reply_count"], 0);
    // B：parent_id=A、reply_to_id null、reply_count=0（两级存储：C 的 parent 是 A 不是 B）
    assert_eq!(items[1]["parent_id"], a_id);
    assert_eq!(items[1]["reply_to_id"], Value::Null);
    assert_eq!(items[1]["reply_count"], 0);
    // A：顶级，reply_count=2（直接子回复条数 = 整线程楼层数）
    assert_eq!(items[2]["parent_id"], Value::Null);
    assert_eq!(items[2]["reply_count"], 2);

    // 隐藏顶级 A → 前台整条线程不可见（子回复一并被过滤），comment_count 同口径归零
    let r = c
        .put(format!("{base}/api/admin/comments/{a_id}"))
        .bearer_auth(&token)
        .json(&json!({"status": "hidden"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    let list = public_comments(&c, &base, slug).await;
    assert!(
        list.is_empty(),
        "hidden 顶级评论的整条线程不应出现在公开列表"
    );
    let v = c
        .get(format!("{base}/api/posts/{slug}"))
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    assert_eq!(v["comment_count"], 0, "comment_count 只统计前台可见评论");
    // 后台仍能看到全部三条（子回复 status 不连带变更）
    let v = c
        .get(format!(
            "{base}/api/admin/comments?post_id={post_id}&status=approved"
        ))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    assert_eq!(v["total"], 2, "子回复 B/C 的 status 不被连带隐藏");

    // 恢复 A → 线程整体重现
    let r = c
        .put(format!("{base}/api/admin/comments/{a_id}"))
        .bearer_auth(&token)
        .json(&json!({"status": "approved"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    assert_eq!(public_comments(&c, &base, slug).await.len(), 3);

    // 删除子回复 B → 只删自身（A、C 仍在；C 的 reply_to_name 仍取自 B 行？B 已删 → JOIN 不到 → null）
    let r = c
        .delete(format!("{base}/api/admin/comments/{b_id}"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 204);
    let list = public_comments(&c, &base, slug).await;
    assert_eq!(list.len(), 2);
    let c_row = list.iter().find(|x| x["id"] == c_id).unwrap();
    assert_eq!(c_row["reply_to_id"], b_id, "reply_to_id 保留（父已删）");
    assert_eq!(
        c_row["reply_to_name"],
        Value::Null,
        "被回复人行已删 → JOIN 不到 → null"
    );

    // 删除顶级 A → 连带删除其全部子回复（C 同删）
    let r = c
        .delete(format!("{base}/api/admin/comments/{a_id}"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 204);
    assert!(public_comments(&c, &base, slug).await.is_empty());
    let v = c
        .get(format!("{base}/api/admin/comments?post_id={post_id}"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    assert_eq!(v["total"], 0, "删顶级评论应连带删除全部子回复");

    // 再删 A → 404（不存在）
    let r = c
        .delete(format!("{base}/api/admin/comments/{a_id}"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 404);
}

// ---------- 2. 留言板留言同一套机制 + 文章↔页面跨目标 422 ----------

#[tokio::test(flavor = "multi_thread")]
async fn guestbook_replies_and_cross_target_type() {
    let tmp = tempfile::tempdir().unwrap();
    let (c, base, token) = setup(tmp.path()).await;

    // 留言板（安装内置页 slug=guestbook）：顶级留言 + 回复 + 回复的回复
    let m = c
        .post(format!("{base}/api/pages/guestbook/comments"))
        .json(&json!({"author_name": "访客", "content": "顶级留言"}))
        .send()
        .await
        .unwrap();
    assert_eq!(m.status(), 201);
    let m = m.json::<Value>().await.unwrap();
    let m_id = m["id"].as_i64().unwrap();
    assert_eq!(m["parent_id"], Value::Null);

    let r1 = c
        .post(format!("{base}/api/pages/guestbook/comments"))
        .json(&json!({"author_name": "站长", "content": "回复留言", "parent_id": m_id}))
        .send()
        .await
        .unwrap();
    assert_eq!(r1.status(), 201);
    let r1 = r1.json::<Value>().await.unwrap();
    let r1_id = r1["id"].as_i64().unwrap();
    assert_eq!(r1["parent_id"], m_id);

    let r2 = c
        .post(format!("{base}/api/pages/guestbook/comments"))
        .json(&json!({"author_name": "访客", "content": "再回复", "parent_id": r1_id}))
        .send()
        .await
        .unwrap();
    assert_eq!(r2.status(), 201);
    let r2 = r2.json::<Value>().await.unwrap();
    assert_eq!(r2["parent_id"], m_id, "留言的回复同样两级归一化");
    assert_eq!(r2["reply_to_id"], r1_id);
    assert_eq!(r2["reply_to_name"], "站长");

    // 留言板公开列表：平铺 ASC + 线程字段
    let v = c
        .get(format!("{base}/api/pages/guestbook/comments"))
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    let list = v.as_array().unwrap();
    assert_eq!(list.len(), 3);
    assert_eq!(list[2]["reply_to_name"], "站长");

    // 跨 target_type：文章评论与留言板留言互不可回复 → 422
    publish_post(&c, &base, &token, "Cross Post").await;
    let p = post_comment(
        &c,
        &base,
        "cross-post",
        json!({"author_name": "读者", "content": "文章评论"}),
    )
    .await;
    let p_id = p["id"].as_i64().unwrap();

    // 在留言板回复文章评论 → 422
    let r = c
        .post(format!("{base}/api/pages/guestbook/comments"))
        .json(&json!({"author_name": "x", "content": "y", "parent_id": p_id}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 422);
    assert_eq!(err_code(r).await, "validation_error");

    // 在文章下回复留言板留言 → 422
    let r = c
        .post(format!("{base}/api/posts/cross-post/comments"))
        .json(&json!({"author_name": "x", "content": "y", "parent_id": m_id}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 422);
    assert_eq!(err_code(r).await, "validation_error");

    // 留言板线程过滤：隐藏顶级留言 → 整条线程不可见；恢复 → 重现
    let r = c
        .put(format!("{base}/api/admin/comments/{m_id}"))
        .bearer_auth(&token)
        .json(&json!({"status": "hidden"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    let v = c
        .get(format!("{base}/api/pages/guestbook/comments"))
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    assert!(v.as_array().unwrap().is_empty());
    let r = c
        .put(format!("{base}/api/admin/comments/{m_id}"))
        .bearer_auth(&token)
        .json(&json!({"status": "approved"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    let v = c
        .get(format!("{base}/api/pages/guestbook/comments"))
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    assert_eq!(v.as_array().unwrap().len(), 3);

    // 删顶级留言 → 连带删除子回复
    let r = c
        .delete(format!("{base}/api/admin/comments/{m_id}"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 204);
    let v = c
        .get(format!("{base}/api/pages/guestbook/comments"))
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    assert!(v.as_array().unwrap().is_empty());
}

// ---------- 3. comment.before_create 钩子对回复生效 ----------

/// 内存构造 zip（与 extensibility.rs 同款）
fn build_zip(files: &[(&str, &str)]) -> Vec<u8> {
    let mut zw = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let opts = zip::write::SimpleFileOptions::default();
    for (name, content) in files {
        zw.start_file(*name, opts).unwrap();
        zw.write_all(content.as_bytes()).unwrap();
    }
    zw.finish().unwrap().into_inner()
}

const REPLY_GUARD_MANIFEST: &str = r#"name = "回复守卫"
slug = "reply-guard"
version = "1.0.0"
description = "只拦截带特定词的回复（验证钩子入参 parent_id/reply_to_id）"
author = "reedblog-test"
hooks = ["comment.before_create"]
"#;

/// 拦截规则：仅当 ctx.parent_id > 0（即回复）且内容含 "ban-reply" 时 block，
/// reason 带上 ctx.parent_id / ctx.reply_to_id 以验证钩子拿到归一化后的值
const REPLY_GUARD_SCRIPT: &str = r#"
fn comment_before_create(ctx) {
    if ctx.parent_id > 0 && ctx.content.contains("ban-reply") {
        #{ action: "block",
           reason: "拦截回复 parent=" + ctx.parent_id + " reply_to=" + ctx.reply_to_id }
    } else {
        #{ action: "allow" }
    }
}
"#;

#[tokio::test(flavor = "multi_thread")]
async fn hook_applies_to_replies_with_normalized_ctx() {
    let tmp = tempfile::tempdir().unwrap();
    let (c, base, token) = setup(tmp.path()).await;

    // 安装并启用插件
    let zip = build_zip(&[
        ("reply-guard/manifest.toml", REPLY_GUARD_MANIFEST),
        ("reply-guard/main.rhai", REPLY_GUARD_SCRIPT),
    ]);
    let part = reqwest::multipart::Part::bytes(zip)
        .file_name("pkg.zip")
        .mime_str("application/zip")
        .unwrap();
    let r = c
        .post(format!("{base}/api/admin/plugins"))
        .bearer_auth(&token)
        .multipart(reqwest::multipart::Form::new().part("file", part))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 201, "插件安装应 201");
    let r = c
        .post(format!("{base}/api/admin/plugins/reply-guard/enable"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200, "插件启用应 200");

    publish_post(&c, &base, &token, "Hooked Post").await;
    let slug = "hooked-post";

    // 顶级评论带 "ban-reply" 也放行（parent_id=0，不是回复）
    let a = post_comment(
        &c,
        &base,
        slug,
        json!({"author_name": "甲", "content": "ban-reply 顶级"}),
    )
    .await;
    let a_id = a["id"].as_i64().unwrap();

    // 回复 + "ban-reply" → 403 comment_blocked，reason 带归一化后的 ctx 值
    let r = c
        .post(format!("{base}/api/posts/{slug}/comments"))
        .json(&json!({"author_name": "乙", "content": "ban-reply 回复", "parent_id": a_id}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 403);
    let (code, msg) = err_code_msg(r).await;
    assert_eq!(code, "comment_blocked");
    assert!(
        msg.contains(&format!("parent={a_id}")) && msg.contains("reply_to=0"),
        "钩子应拿到归一化后的 ctx.parent_id/reply_to_id: {msg}"
    );

    // 正常回复放行 → 再用它验证「回复的回复」时 ctx.reply_to_id 为中间楼层 id
    let b = post_comment(
        &c,
        &base,
        slug,
        json!({"author_name": "乙", "content": "正常回复", "parent_id": a_id}),
    )
    .await;
    let b_id = b["id"].as_i64().unwrap();
    let r = c
        .post(format!("{base}/api/posts/{slug}/comments"))
        .json(&json!({"author_name": "丙", "content": "ban-reply 深层", "parent_id": b_id}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 403);
    let msg = err_code_msg(r).await.1;
    assert!(
        msg.contains(&format!("parent={a_id}")) && msg.contains(&format!("reply_to={b_id}")),
        "深层回复的 ctx 应为归一化后的 parent=顶级/reply_to=中间楼层: {msg}"
    );

    // 留言板留言的回复同样被钩子拦截（同一管线）
    let m = c
        .post(format!("{base}/api/pages/guestbook/comments"))
        .json(&json!({"author_name": "访客", "content": "留言"}))
        .send()
        .await
        .unwrap();
    let m_id = m.json::<Value>().await.unwrap()["id"].as_i64().unwrap();
    let r = c
        .post(format!("{base}/api/pages/guestbook/comments"))
        .json(&json!({"author_name": "x", "content": "ban-reply", "parent_id": m_id}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 403);
    assert_eq!(err_code(r).await, "comment_blocked");

    // 停用插件后回复恢复放行
    let r = c
        .post(format!("{base}/api/admin/plugins/reply-guard/disable"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    let d = post_comment(
        &c,
        &base,
        slug,
        json!({"author_name": "丙", "content": "ban-reply 停用后", "parent_id": b_id}),
    )
    .await;
    assert_eq!(d["parent_id"], a_id);
    assert_eq!(d["reply_to_id"], b_id);
}
