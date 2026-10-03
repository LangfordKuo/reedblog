//! 备份与恢复集成测试（api-contract.md「备份与恢复」条款，2026-10-04 新增）：
//! - 导出：zip 可解压，manifest 表计数与库内一致、data.json 含预期表与行、
//!   uploads 条目在内且内容字节一致、回收站文章也在备份里
//! - 恢复：改标题/加文章/删媒体/删评论 → 导入备份 → 数据回到备份时点（含媒体文件、id 不变）
//! - 失败语义：缺 confirm → 422 且数据不变；每个非法 zip 场景 → 422 且现有数据不变；
//!   `uploads/../../evil.txt` → 422 且磁盘外不产生文件；无鉴权 → 401
//! - 导入后原管理员账号仍可登录（密码哈希随备份恢复）

use reedblog_backend::state::AppState;
use serde_json::{json, Value};
use std::io::{Cursor, Read, Write};
use std::path::{Path, PathBuf};

/// 在 127.0.0.1 随机端口起真实服务；预写 config.toml 把插件/主题/上传目录隔离到 tempdir
async fn spawn_server(config_path: &str) -> String {
    if !Path::new(config_path).exists() {
        let dir = Path::new(config_path).parent().unwrap();
        let toml_path = |p: PathBuf| p.to_str().unwrap().replace('\\', "/");
        std::fs::write(
            config_path,
            format!(
                "[plugins]\ndir = \"{}\"\n\n[themes]\ndir = \"{}\"\n\n[uploads]\ndir = \"{}\"\n",
                toml_path(dir.join("plugins")),
                toml_path(dir.join("themes")),
                toml_path(dir.join("uploads")),
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

/// 安装 + 登录，返回 Bearer token
async fn setup_installed(c: &reqwest::Client, base: &str, dir: &Path) -> String {
    let r = c
        .post(format!("{base}/api/install"))
        .json(&json!({
            "db_type": "sqlite",
            "sqlite_path": dir.join("reedblog.db").to_str().unwrap(),
            "admin": {"username": "admin", "password": "secret123"},
            "site": {"title": "备份测试博客"}
        }))
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
    assert_eq!(r.status(), 200, "管理员登录应成功");
    r.json::<Value>().await.unwrap()["token"]
        .as_str()
        .unwrap()
        .to_string()
}

/// 构造带 IHDR 的最小 PNG 头（可被 magic bytes 与宽高解析识别；测试不真正解码）
fn tiny_png(pad: usize) -> Vec<u8> {
    let mut v = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
    v.extend_from_slice(&13u32.to_be_bytes());
    v.extend_from_slice(b"IHDR");
    v.extend_from_slice(&1u32.to_be_bytes());
    v.extend_from_slice(&1u32.to_be_bytes());
    v.extend_from_slice(&[8, 6, 0, 0, 0]);
    v.resize(v.len() + pad, 0xAB);
    v
}

async fn upload_image(c: &reqwest::Client, base: &str, token: &str, data: Vec<u8>) -> Value {
    let part = reqwest::multipart::Part::bytes(data).file_name("pic.png".to_string());
    let form = reqwest::multipart::Form::new().part("file", part);
    let r = c
        .post(format!("{base}/api/admin/uploads"))
        .bearer_auth(token)
        .multipart(form)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    r.json().await.unwrap()
}

async fn create_post(c: &reqwest::Client, base: &str, token: &str, title: &str) -> Value {
    let r = c
        .post(format!("{base}/api/admin/posts"))
        .bearer_auth(token)
        .json(&json!({"title": title, "content_md": "正文内容", "status": "published"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 201, "发文失败: {}", r.text().await.unwrap());
    r.json().await.unwrap()
}

async fn admin_posts(c: &reqwest::Client, base: &str, token: &str) -> Vec<Value> {
    let r = c
        .get(format!("{base}/api/admin/posts?status=all&per_page=100"))
        .bearer_auth(token)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    r.json::<Value>().await.unwrap()["items"]
        .as_array()
        .unwrap()
        .clone()
}

async fn export(c: &reqwest::Client, base: &str, token: Option<&str>) -> reqwest::Response {
    let mut req = c.get(format!("{base}/api/admin/backup/export"));
    if let Some(t) = token {
        req = req.bearer_auth(t);
    }
    req.send().await.unwrap()
}

async fn import(
    c: &reqwest::Client,
    base: &str,
    token: Option<&str>,
    zip: Vec<u8>,
    confirm: Option<&str>,
) -> reqwest::Response {
    let part = reqwest::multipart::Part::bytes(zip)
        .file_name("backup.zip".to_string())
        .mime_str("application/zip")
        .unwrap();
    let mut form = reqwest::multipart::Form::new().part("file", part);
    if let Some(cf) = confirm {
        form = form.text("confirm", cf.to_string());
    }
    let mut req = c
        .post(format!("{base}/api/admin/backup/import"))
        .multipart(form);
    if let Some(t) = token {
        req = req.bearer_auth(t);
    }
    req.send().await.unwrap()
}

/// 用测试数据构造 zip（可用于非法包场景）
fn zip_bytes(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let opts = zip::write::SimpleFileOptions::default();
    for (name, data) in entries {
        writer.start_file(*name, opts).unwrap();
        writer.write_all(data).unwrap();
    }
    writer.finish().unwrap().into_inner()
}

fn zip_entry(zip: &[u8], name: &str) -> Option<Vec<u8>> {
    let mut archive = zip::ZipArchive::new(Cursor::new(zip)).unwrap();
    let mut f = archive.by_name(name).ok()?;
    let mut buf = Vec::new();
    f.read_to_end(&mut buf).unwrap();
    Some(buf)
}

fn zip_names(zip: &[u8]) -> Vec<String> {
    let mut archive = zip::ZipArchive::new(Cursor::new(zip)).unwrap();
    (0..archive.len())
        .map(|i| archive.by_index(i).unwrap().name().to_string())
        .collect()
}

fn json_entry(zip: &[u8], name: &str) -> Value {
    serde_json::from_slice(&zip_entry(zip, name).unwrap_or_else(|| panic!("缺少 {name}"))).unwrap()
}

// ---------- 1. 导出：zip 结构 / 计数 / 行数据 / uploads 字节一致 ----------

#[tokio::test(flavor = "multi_thread")]
async fn export_produces_expected_zip() {
    let tmp = tempfile::tempdir().unwrap();
    let base = spawn_server(tmp.path().join("config.toml").to_str().unwrap()).await;
    let c = reqwest::Client::new();
    let token = setup_installed(&c, &base, tmp.path()).await;

    // 造数据：一篇文章 + 一张图 + 一条评论；另建一篇软删（回收站）文章
    let post = create_post(&c, &base, &token, "备份时点的标题").await;
    let slug = post["slug"].as_str().unwrap().to_string();
    let post_id = post["id"].as_i64().unwrap();
    let png = tiny_png(2048);
    let media = upload_image(&c, &base, &token, png.clone()).await;
    let media_id = media["id"].as_i64().unwrap();
    let media_url = media["url"].as_str().unwrap().to_string();

    let r = c
        .post(format!("{base}/api/posts/{slug}/comments"))
        .json(&json!({"author_name": "访客", "content": "备份时点的评论"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 201);

    let trash_post = create_post(&c, &base, &token, "回收站里的文章").await;
    let r = c
        .delete(format!(
            "{base}/api/admin/posts/{}",
            trash_post["id"].as_i64().unwrap()
        ))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 204);

    // 导出
    let r = export(&c, &base, Some(&token)).await;
    assert_eq!(r.status(), 200);
    assert_eq!(
        r.headers().get("content-type").unwrap().to_str().unwrap(),
        "application/zip"
    );
    let disposition = r
        .headers()
        .get("content-disposition")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    assert!(
        disposition.contains("attachment; filename=\"reedblog-backup-"),
        "{disposition}"
    );
    assert!(disposition.contains(".zip\""), "{disposition}");
    let zip = r.bytes().await.unwrap().to_vec();

    // zip 结构：manifest.json + data.json + uploads/...（仅这三类）
    let names = zip_names(&zip);
    assert!(names.contains(&"manifest.json".to_string()));
    assert!(names.contains(&"data.json".to_string()));
    let upload_names: Vec<&String> = names.iter().filter(|n| n.starts_with("uploads/")).collect();
    assert_eq!(upload_names.len(), 1, "应恰好包含上传的这一张图: {names:?}");

    // manifest：版本/库类型/表计数与库内一致
    let manifest = json_entry(&zip, "manifest.json");
    assert_eq!(manifest["format_version"], 1);
    assert_eq!(manifest["db_type"], "sqlite");
    let tables = manifest["tables"].as_object().unwrap();
    for t in [
        "users",
        "categories",
        "tags",
        "posts",
        "post_tags",
        "comments",
        "plugins",
        "settings",
        "pages",
        "page_links",
        "theme_settings",
        "theme_widgets",
        "post_likes",
        "media",
        "post_revisions",
    ] {
        assert!(tables.contains_key(t), "manifest.tables 缺少 {t}");
    }
    assert_eq!(manifest["media_files"], 1);
    // 导出含回收站文章：manifest 计数 = 正常文章 + 回收站文章
    let db_posts = admin_posts(&c, &base, &token).await;
    let trash: Value = c
        .get(format!("{base}/api/admin/posts/trash?per_page=100"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        tables["posts"].as_i64().unwrap(),
        (db_posts.len() + trash["total"].as_u64().unwrap() as usize) as i64
    );
    let media_list: Value = c
        .get(format!("{base}/api/admin/media?per_page=100"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        tables["media"].as_i64().unwrap(),
        media_list["total"].as_i64().unwrap()
    );

    // data.json：含预期表与行；行键=列名、时间戳保持 RFC3339 文本、密码哈希在内
    let data = json_entry(&zip, "data.json");
    let posts = data["posts"].as_array().unwrap();
    let our_post = posts
        .iter()
        .find(|p| p["id"] == post_id)
        .expect("备份应包含该文章");
    assert_eq!(our_post["title"], "备份时点的标题");
    assert!(our_post["created_at"].as_str().unwrap().ends_with('Z'));
    // 回收站文章（deleted_at 非空）同样在备份里，修订行也在
    assert!(posts
        .iter()
        .any(|p| p["title"] == "回收站里的文章" && p["deleted_at"].is_string()));
    assert!(data["post_revisions"]
        .as_array()
        .unwrap()
        .iter()
        .any(|r| r["post_id"] == post_id));
    let users = data["users"].as_array().unwrap();
    assert!(users[0]["password_hash"]
        .as_str()
        .unwrap()
        .starts_with("$argon2"));
    assert!(data["comments"]
        .as_array()
        .unwrap()
        .iter()
        .any(|c| c["content"] == "备份时点的评论"));

    // uploads 条目内容字节一致（含相对路径保持）
    let url_path = media_url.strip_prefix("/api/uploads/").unwrap();
    let stored = zip_entry(&zip, &format!("uploads/{url_path}")).expect("uploads 条目缺失");
    assert_eq!(stored, png, "媒体文件字节应原样复制");

    // info：最近导出信息（内存态）
    let info: Value = c
        .get(format!("{base}/api/admin/backup/info"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(info["last_export_at"].as_str().unwrap().ends_with('Z'));
    assert_eq!(info["total_size_bytes"].as_u64().unwrap(), zip.len() as u64);

    // 媒体删除在导出之后：不影响的清理（保持临时目录整洁）
    let _ = media_id;
}

// ---------- 2. 恢复：数据与媒体回到备份时点 ----------

#[tokio::test(flavor = "multi_thread")]
async fn import_restores_data_and_media() {
    let tmp = tempfile::tempdir().unwrap();
    let base = spawn_server(tmp.path().join("config.toml").to_str().unwrap()).await;
    let c = reqwest::Client::new();
    let token = setup_installed(&c, &base, tmp.path()).await;

    let post = create_post(&c, &base, &token, "备份时标题").await;
    let post_id = post["id"].as_i64().unwrap();
    let slug = post["slug"].as_str().unwrap().to_string();
    let png = tiny_png(4096);
    let media = upload_image(&c, &base, &token, png.clone()).await;
    let media_id = media["id"].as_i64().unwrap();
    let media_url = media["url"].as_str().unwrap().to_string();
    let comment: Value = c
        .post(format!("{base}/api/posts/{slug}/comments"))
        .json(&json!({"author_name": "访客", "content": "备份时评论"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let comment_id = comment["id"].as_i64().unwrap();
    let posts_before = admin_posts(&c, &base, &token).await.len();

    // 备份
    let r = export(&c, &base, Some(&token)).await;
    assert_eq!(r.status(), 200);
    let zip = r.bytes().await.unwrap().to_vec();

    // 备份后新增一张不同的图：导入应把它连同磁盘文件一起清掉（磁盘回到备份时点）
    let png2 = tiny_png(1024);
    let media2 = upload_image(&c, &base, &token, png2).await;
    let media2_url = media2["url"].as_str().unwrap().to_string();
    assert_ne!(media2_url, media_url, "内容不同应生成不同存储路径");

    // 改成另一种状态：改标题、加文章、删媒体、删评论
    let r = c
        .put(format!("{base}/api/admin/posts/{post_id}"))
        .bearer_auth(&token)
        .json(&json!({"title": "被改动的标题"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    create_post(&c, &base, &token, "备份后新增的文章").await;
    let r = c
        .delete(format!("{base}/api/admin/media/{media_id}"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 204);
    let r = c
        .delete(format!("{base}/api/admin/comments/{comment_id}"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 204);
    // 媒体文件已从磁盘删除
    let r = c.get(format!("{base}{media_url}")).send().await.unwrap();
    assert_eq!(r.status(), 404);

    // 导入（confirm=REPLACE）
    let r = import(&c, &base, Some(&token), zip.clone(), Some("REPLACE")).await;
    let status = r.status();
    let body: Value = r.json().await.unwrap();
    assert_eq!(status, 200, "导入应成功: {body}");
    assert_eq!(body["ok"], true);
    assert_eq!(body["format_version"], 1);
    assert_eq!(body["media_files"], 1);
    assert_eq!(
        body["tables"]["posts"].as_i64().unwrap(),
        posts_before as i64
    );

    // 数据回到备份时点：标题恢复、id 不变、新增文章消失、评论恢复
    let posts = admin_posts(&c, &base, &token).await;
    assert_eq!(posts.len(), posts_before, "文章数应回到备份时点");
    let restored = posts
        .iter()
        .find(|p| p["id"] == post_id)
        .expect("原文章应存在且 id 不变");
    assert_eq!(restored["title"], "备份时标题");
    assert!(!posts.iter().any(|p| p["title"] == "备份后新增的文章"));
    let comments: Value = c
        .get(format!("{base}/api/admin/comments?per_page=100"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(comments["items"]
        .as_array()
        .unwrap()
        .iter()
        .any(|cm| cm["id"] == comment_id && cm["content"] == "备份时评论"));

    // 媒体文件回到磁盘 + media 记录恢复
    let r = c.get(format!("{base}{media_url}")).send().await.unwrap();
    assert_eq!(r.status(), 200, "媒体文件应随备份恢复");
    assert_eq!(r.bytes().await.unwrap().to_vec(), png);
    let media_list: Value = c
        .get(format!("{base}/api/admin/media?per_page=100"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(media_list["total"], 1);
    assert_eq!(media_list["items"][0]["id"], media_id);

    // 备份后新增的媒体（记录 + 磁盘文件）已被清理：磁盘与备份时点完全一致
    let r = c.get(format!("{base}{media2_url}")).send().await.unwrap();
    assert_eq!(r.status(), 404, "备份外的新媒体文件应被清理");
    let mut file_count = 0usize;
    let mut stack = vec![tmp.path().join("uploads")];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in entries.flatten() {
            let ft = e.file_type().unwrap();
            if ft.is_dir() {
                stack.push(e.path());
            } else if ft.is_file() {
                file_count += 1;
            }
        }
    }
    assert_eq!(file_count, 1, "uploads 目录应只剩下备份时点的那 1 个文件");

    // 导入后原管理员账号仍可登录（users 表随备份恢复，密码哈希一致）
    let _ = login(&c, &base).await;
}

// ---------- 3. confirm 校验：缺失/错误 → 422 且数据不变 ----------

#[tokio::test(flavor = "multi_thread")]
async fn import_requires_replace_confirmation() {
    let tmp = tempfile::tempdir().unwrap();
    let base = spawn_server(tmp.path().join("config.toml").to_str().unwrap()).await;
    let c = reqwest::Client::new();
    let token = setup_installed(&c, &base, tmp.path()).await;

    // 未导出过：info 两个字段均为 null
    let info: Value = c
        .get(format!("{base}/api/admin/backup/info"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(info["last_export_at"].is_null());
    assert!(info["total_size_bytes"].is_null());

    let post = create_post(&c, &base, &token, "原始标题").await;
    let post_id = post["id"].as_i64().unwrap();
    let r = export(&c, &base, Some(&token)).await;
    let zip = r.bytes().await.unwrap().to_vec();
    // 改成可区分的状态
    c.put(format!("{base}/api/admin/posts/{post_id}"))
        .bearer_auth(&token)
        .json(&json!({"title": "改动后的标题"}))
        .send()
        .await
        .unwrap();

    // 缺 confirm → 422 confirmation_required，数据不变
    let r = import(&c, &base, Some(&token), zip.clone(), None).await;
    assert_eq!(r.status(), 422);
    assert_eq!(err_code(r).await, "confirmation_required");
    // confirm 值错误（小写）同样拒绝
    let r = import(&c, &base, Some(&token), zip.clone(), Some("replace")).await;
    assert_eq!(r.status(), 422);
    assert_eq!(err_code(r).await, "confirmation_required");
    let posts = admin_posts(&c, &base, &token).await;
    assert!(posts.iter().any(|p| p["title"] == "改动后的标题"));

    // 正确 confirm → 成功，标题回到备份时点（证明前两次确实未落库）
    let r = import(&c, &base, Some(&token), zip, Some("REPLACE")).await;
    assert_eq!(r.status(), 200);
    let posts = admin_posts(&c, &base, &token).await;
    assert!(posts.iter().any(|p| p["title"] == "原始标题"));
}

// ---------- 4. 非法 zip：全部 422 且现有数据/磁盘外不变 ----------

#[tokio::test(flavor = "multi_thread")]
async fn import_rejects_invalid_zips() {
    let tmp = tempfile::tempdir().unwrap();
    let base = spawn_server(tmp.path().join("config.toml").to_str().unwrap()).await;
    let c = reqwest::Client::new();
    let token = setup_installed(&c, &base, tmp.path()).await;

    let post = create_post(&c, &base, &token, "原始标题").await;
    let post_id = post["id"].as_i64().unwrap();
    let r = export(&c, &base, Some(&token)).await;
    let good_zip = r.bytes().await.unwrap().to_vec();
    // 改动状态：所有失败导入都不得把它改回去
    c.put(format!("{base}/api/admin/posts/{post_id}"))
        .bearer_auth(&token)
        .json(&json!({"title": "改动后的标题"}))
        .send()
        .await
        .unwrap();

    // 4.1 非 zip 字节
    let r = import(
        &c,
        &base,
        Some(&token),
        b"this is not a zip".to_vec(),
        Some("REPLACE"),
    )
    .await;
    assert_eq!(r.status(), 422);
    assert_eq!(err_code(r).await, "invalid_backup");

    // 4.2 空 zip（无 manifest / 无 data.json）
    let empty = zip_bytes(&[]);
    let r = import(&c, &base, Some(&token), empty, Some("REPLACE")).await;
    assert_eq!(r.status(), 422);
    assert_eq!(err_code(r).await, "invalid_backup");

    // 4.3 zip 里塞 `uploads/../../evil.txt` → 422 且磁盘外不产生文件
    let manifest = json_entry(&good_zip, "manifest.json");
    let data = json_entry(&good_zip, "data.json");
    let evil = zip_bytes(&[
        ("manifest.json", manifest.to_string().as_bytes()),
        ("data.json", data.to_string().as_bytes()),
        ("uploads/../../evil.txt", b"pwned"),
    ]);
    let r = import(&c, &base, Some(&token), evil, Some("REPLACE")).await;
    assert_eq!(r.status(), 422);
    assert_eq!(err_code(r).await, "invalid_backup");
    assert!(
        !tmp.path().join("evil.txt").exists(),
        "越界路径不得在 uploads 根外产生文件"
    );
    assert!(!tmp
        .path()
        .join("uploads")
        .join("..")
        .join("evil.txt")
        .exists());

    // 4.4 format_version 不兼容
    let mut bad_manifest = manifest.clone();
    bad_manifest["format_version"] = json!(99);
    let bad = zip_bytes(&[
        ("manifest.json", bad_manifest.to_string().as_bytes()),
        ("data.json", data.to_string().as_bytes()),
    ]);
    let r = import(&c, &base, Some(&token), bad, Some("REPLACE")).await;
    assert_eq!(r.status(), 422);
    assert_eq!(err_code(r).await, "invalid_backup");

    // 4.5 manifest 表计数与 data.json 不一致
    let mut bad_manifest = manifest.clone();
    let posts_count = bad_manifest["tables"]["posts"].as_i64().unwrap();
    bad_manifest["tables"]["posts"] = json!(posts_count + 1);
    let bad = zip_bytes(&[
        ("manifest.json", bad_manifest.to_string().as_bytes()),
        ("data.json", data.to_string().as_bytes()),
    ]);
    let r = import(&c, &base, Some(&token), bad, Some("REPLACE")).await;
    assert_eq!(r.status(), 422);
    assert_eq!(err_code(r).await, "invalid_backup");

    // 4.6 data.json 含允许集合外的表名
    let mut bad_data = data.clone();
    bad_data["evil_table"] = json!([]);
    let mut bad_manifest = manifest.clone();
    bad_manifest["tables"]["evil_table"] = json!(0);
    let bad = zip_bytes(&[
        ("manifest.json", bad_manifest.to_string().as_bytes()),
        ("data.json", bad_data.to_string().as_bytes()),
    ]);
    let r = import(&c, &base, Some(&token), bad, Some("REPLACE")).await;
    assert_eq!(r.status(), 422);
    assert_eq!(err_code(r).await, "invalid_backup");

    // 4.7 行值类型不符（posts.id 给字符串）
    let mut bad_data = data.clone();
    if let Some(row) = bad_data["posts"].as_array_mut().and_then(|a| a.first_mut()) {
        row["id"] = json!("not-a-number");
    }
    let bad = zip_bytes(&[
        ("manifest.json", manifest.to_string().as_bytes()),
        ("data.json", bad_data.to_string().as_bytes()),
    ]);
    let r = import(&c, &base, Some(&token), bad, Some("REPLACE")).await;
    assert_eq!(r.status(), 422);
    assert_eq!(err_code(r).await, "invalid_backup");

    // 4.8 zip 含未知条目
    let bad = zip_bytes(&[
        ("manifest.json", manifest.to_string().as_bytes()),
        ("data.json", data.to_string().as_bytes()),
        ("evil.txt", b"x"),
    ]);
    let r = import(&c, &base, Some(&token), bad, Some("REPLACE")).await;
    assert_eq!(r.status(), 422);
    assert_eq!(err_code(r).await, "invalid_backup");

    // 所有失败之后：数据仍是改动后的状态（未被任何一次失败导入影响）
    let posts = admin_posts(&c, &base, &token).await;
    assert!(posts.iter().any(|p| p["title"] == "改动后的标题"));

    // 合法备份仍能成功导入（对照，证明失败均为校验拦截而非常规路径损坏）
    let r = import(&c, &base, Some(&token), good_zip, Some("REPLACE")).await;
    assert_eq!(r.status(), 200);
    let posts = admin_posts(&c, &base, &token).await;
    assert!(posts.iter().any(|p| p["title"] == "原始标题"));
}

// ---------- 5. 鉴权 ----------

#[tokio::test(flavor = "multi_thread")]
async fn backup_endpoints_require_auth() {
    let tmp = tempfile::tempdir().unwrap();
    let base = spawn_server(tmp.path().join("config.toml").to_str().unwrap()).await;
    let c = reqwest::Client::new();
    let _token = setup_installed(&c, &base, tmp.path()).await;

    let r = export(&c, &base, None).await;
    assert_eq!(r.status(), 401);
    assert_eq!(err_code(r).await, "unauthorized");

    let r = import(&c, &base, None, b"whatever".to_vec(), Some("REPLACE")).await;
    assert_eq!(r.status(), 401);
    assert_eq!(err_code(r).await, "unauthorized");

    let r = c
        .get(format!("{base}/api/admin/backup/info"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 401);
    assert_eq!(err_code(r).await, "unauthorized");
}
