//! 媒体库集成测试（api-contract.md「媒体库」条款，2026-10-04 新增）：
//! - 上传后出现在列表（id/原始文件名/size/mime/宽高/created_at），响应新增 id
//! - 同图重复上传不新增行（返回同一 id/url）
//! - 分页（total/page/per_page/回页序）
//! - 删除：记录消失 + 磁盘文件消失（旧 URL 404）；文件缺失幂等；不存在 id → 404
//! - 非法路径（穿越/盘符）不能删除 uploads 目录外的文件
//! - 无鉴权 → 401
//! - 历史文件（直接写入 uploads 目录）由列表惰性扫描兜底可见；非图片文件不入库

use reedblog_backend::state::AppState;
use serde_json::{json, Value};
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
            "site": {"title": "媒体库测试博客"}
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

/// multipart 上传（字段名 file；token=None 模拟未登录）
async fn upload(
    c: &reqwest::Client,
    base: &str,
    token: Option<&str>,
    filename: &str,
    data: Vec<u8>,
) -> reqwest::Response {
    let part = reqwest::multipart::Part::bytes(data).file_name(filename.to_string());
    let form = reqwest::multipart::Form::new().part("file", part);
    let mut req = c.post(format!("{base}/api/admin/uploads")).multipart(form);
    if let Some(t) = token {
        req = req.bearer_auth(t);
    }
    req.send().await.unwrap()
}

/// GET 媒体列表（可带 token），返回响应
async fn list(
    c: &reqwest::Client,
    base: &str,
    token: Option<&str>,
    query: &str,
) -> reqwest::Response {
    let mut req = c.get(format!("{base}/api/admin/media{query}"));
    if let Some(t) = token {
        req = req.bearer_auth(t);
    }
    req.send().await.unwrap()
}

/// 构造带 IHDR 的最小 PNG 头：宽高可被头解析读出（其余填充为零，测试不真正解码）
fn png_with_dims(w: u32, h: u32, pad: usize) -> Vec<u8> {
    let mut v = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
    v.extend_from_slice(&13u32.to_be_bytes());
    v.extend_from_slice(b"IHDR");
    v.extend_from_slice(&w.to_be_bytes());
    v.extend_from_slice(&h.to_be_bytes());
    v.extend_from_slice(&[8, 6, 0, 0, 0]);
    v.resize(v.len() + pad, 0);
    v
}

/// 直连 SQLite 写库（测非法 url 行的删除防护）；url 与连接串口径同 Config::db_url
async fn insert_media_row(dir: &Path, url: &str) {
    sqlx::any::install_default_drivers();
    let db = dir.join("reedblog.db").to_str().unwrap().replace('\\', "/");
    // Windows 盘符冒号百分号编码（同 Config::db_url 的 encode_sqlite_path）
    let db = db.replacen(':', "%3A", 1);
    let pool = sqlx::AnyPool::connect(&format!("sqlite://{db}?mode=rwc"))
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO media (url, filename, size, mime, width, height, created_at) \
         VALUES (?, 'evil.png', 3, 'image/png', NULL, NULL, '2026-10-04T00:00:00Z')",
    )
    .bind(url)
    .execute(&pool)
    .await
    .unwrap();
}

// ---------- 1. 上传 → 列表可见（形状/宽高/原始文件名） + 静态读取 ----------

#[tokio::test(flavor = "multi_thread")]
async fn uploaded_image_appears_in_media_list() {
    let tmp = tempfile::tempdir().unwrap();
    let base = spawn_server(tmp.path().join("config.toml").to_str().unwrap()).await;
    let c = reqwest::Client::new();
    let token = setup_installed(&c, &base, tmp.path()).await;

    let data = png_with_dims(3, 2, 4);
    let r = upload(&c, &base, Some(&token), "我的 图.png", data.clone()).await;
    assert_eq!(r.status(), 200);
    let up: Value = r.json().await.unwrap();
    assert!(up["id"].is_i64(), "上传响应应含 media id：{up}");
    assert_eq!(up["size"], data.len() as i64);
    assert_eq!(up["filename"], "我的 图.png", "filename 仍回显原始文件名");
    let upload_id = up["id"].as_i64().unwrap();

    let r = list(&c, &base, Some(&token), "").await;
    assert_eq!(r.status(), 200);
    let page: Value = r.json().await.unwrap();
    assert_eq!(page["total"], 1);
    assert_eq!(page["page"], 1);
    assert_eq!(page["per_page"], 10, "缺省分页口径同 normalize_paging");
    let item = &page["items"][0];
    assert_eq!(item["id"].as_i64().unwrap(), upload_id);
    assert_eq!(item["url"], up["url"]);
    assert_eq!(item["filename"], "我的 图.png");
    assert_eq!(item["size"], data.len() as i64);
    assert_eq!(item["mime"], "image/png");
    assert_eq!(item["width"], 3, "PNG 宽应由图片头解析");
    assert_eq!(item["height"], 2);
    assert!(
        item["created_at"].as_str().unwrap().len() >= 20,
        "RFC3339 时间戳"
    );

    // 图片静态读取仍可用（上传流程未破坏）
    let rel = up["url"].as_str().unwrap();
    let r = c.get(format!("{base}{rel}")).send().await.unwrap();
    assert_eq!(r.status(), 200);
    assert_eq!(r.headers()["content-type"], "image/png");
}

// ---------- 2. 同图重复上传不新增行（返回既有 id/url） ----------

#[tokio::test(flavor = "multi_thread")]
async fn duplicate_upload_reuses_record() {
    let tmp = tempfile::tempdir().unwrap();
    let base = spawn_server(tmp.path().join("config.toml").to_str().unwrap()).await;
    let c = reqwest::Client::new();
    let token = setup_installed(&c, &base, tmp.path()).await;

    let data = png_with_dims(5, 5, 8);
    let a: Value = upload(&c, &base, Some(&token), "first.png", data.clone())
        .await
        .json()
        .await
        .unwrap();
    let b: Value = upload(&c, &base, Some(&token), "second.png", data.clone())
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(a["url"], b["url"], "同内容复用同一存储路径");
    assert_eq!(a["id"], b["id"], "重复上传不新增 media 行，返回既有 id");

    let page: Value = list(&c, &base, Some(&token), "")
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(page["total"], 1, "重复上传后列表仍只有一行");
}

// ---------- 3. 分页 ----------

#[tokio::test(flavor = "multi_thread")]
async fn media_list_paginates() {
    let tmp = tempfile::tempdir().unwrap();
    let base = spawn_server(tmp.path().join("config.toml").to_str().unwrap()).await;
    let c = reqwest::Client::new();
    let token = setup_installed(&c, &base, tmp.path()).await;

    let mut ids = Vec::new();
    for i in 0..3u32 {
        let v: Value = upload(
            &c,
            &base,
            Some(&token),
            &format!("p{i}.png"),
            png_with_dims(4 + i, 4, i as usize),
        )
        .await
        .json()
        .await
        .unwrap();
        ids.push(v["id"].as_i64().unwrap());
    }

    let p1: Value = list(&c, &base, Some(&token), "?page=1&per_page=2")
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(p1["total"], 3);
    assert_eq!(p1["per_page"], 2);
    assert_eq!(p1["items"].as_array().unwrap().len(), 2);
    // created_at DESC, id DESC：同秒上传时后传的（id 大）在前
    assert_eq!(p1["items"][0]["id"].as_i64().unwrap(), ids[2]);
    assert_eq!(p1["items"][1]["id"].as_i64().unwrap(), ids[1]);

    let p2: Value = list(&c, &base, Some(&token), "?page=2&per_page=2")
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(p2["items"].as_array().unwrap().len(), 1);
    assert_eq!(p2["items"][0]["id"].as_i64().unwrap(), ids[0]);

    // per_page 上限 100（normalize_paging 口径）
    let clamped: Value = list(&c, &base, Some(&token), "?per_page=999")
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(clamped["per_page"], 100);
}

// ---------- 4. 删除：记录消失 + 磁盘文件消失；幂等；不存在 → 404 ----------

#[tokio::test(flavor = "multi_thread")]
async fn delete_removes_record_and_file() {
    let tmp = tempfile::tempdir().unwrap();
    let base = spawn_server(tmp.path().join("config.toml").to_str().unwrap()).await;
    let c = reqwest::Client::new();
    let token = setup_installed(&c, &base, tmp.path()).await;

    let up: Value = upload(&c, &base, Some(&token), "gone.png", png_with_dims(6, 7, 12))
        .await
        .json()
        .await
        .unwrap();
    let id = up["id"].as_i64().unwrap();
    let url = up["url"].as_str().unwrap().to_string();
    // 落盘文件确实存在（url 去掉 /api/uploads/ 前缀即相对路径）
    let disk_path = tmp.path().join("uploads").join(
        url.strip_prefix("/api/uploads/")
            .unwrap()
            .replace('/', std::path::MAIN_SEPARATOR_STR),
    );
    assert!(disk_path.is_file(), "上传后文件应落盘: {disk_path:?}");

    let r = c
        .delete(format!("{base}/api/admin/media/{id}"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 204);
    assert!(!disk_path.exists(), "删除后磁盘文件应消失");
    let r = c.get(format!("{base}{url}")).send().await.unwrap();
    assert_eq!(r.status(), 404, "删除后旧 URL 应 404");

    let page: Value = list(&c, &base, Some(&token), "")
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(page["total"], 0, "删除后记录消失");

    // 再删同一 id → 404
    let r = c
        .delete(format!("{base}/api/admin/media/{id}"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 404);
    assert_eq!(err_code(r).await, "not_found");

    // 文件缺失幂等：先手动删文件，再删记录 → 204 且记录消失
    let up2: Value = upload(&c, &base, Some(&token), "lost.png", png_with_dims(9, 9, 3))
        .await
        .json()
        .await
        .unwrap();
    let id2 = up2["id"].as_i64().unwrap();
    let disk2 = tmp.path().join("uploads").join(
        up2["url"]
            .as_str()
            .unwrap()
            .strip_prefix("/api/uploads/")
            .unwrap()
            .replace('/', std::path::MAIN_SEPARATOR_STR),
    );
    std::fs::remove_file(&disk2).unwrap();
    let r = c
        .delete(format!("{base}/api/admin/media/{id2}"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 204, "文件缺失时删记录仍幂等成功");
    let page: Value = list(&c, &base, Some(&token), "")
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(page["total"], 0);
}

// ---------- 5. 非法路径不能穿越删除 uploads 目录外的文件 ----------

#[tokio::test(flavor = "multi_thread")]
async fn delete_cannot_escape_uploads_dir() {
    let tmp = tempfile::tempdir().unwrap();
    let base = spawn_server(tmp.path().join("config.toml").to_str().unwrap()).await;
    let c = reqwest::Client::new();
    let token = setup_installed(&c, &base, tmp.path()).await;

    // uploads 根目录之外的受害者文件
    let victim = tmp.path().join("secret.txt");
    std::fs::write(&victim, b"do not delete").unwrap();

    // 绕过 API 直接塞入非法 url 的 media 行（模拟被篡改的记录）
    insert_media_row(tmp.path(), "/api/uploads/../../secret.txt").await;
    insert_media_row(tmp.path(), "/api/uploads/2026/../../secret.txt").await;
    insert_media_row(tmp.path(), "/api/uploads/C:/secret.txt").await;

    let page: Value = list(&c, &base, Some(&token), "")
        .await
        .json()
        .await
        .unwrap();
    let ids: Vec<i64> = page["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|it| it["id"].as_i64().unwrap())
        .collect();
    assert_eq!(ids.len(), 3, "3 条恶意行都应可见（列表只读）");

    for id in ids {
        let r = c
            .delete(format!("{base}/api/admin/media/{id}"))
            .bearer_auth(&token)
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), 204);
    }
    assert!(victim.is_file(), "uploads 目录外的文件绝不能被删除");
    assert_eq!(std::fs::read(&victim).unwrap(), b"do not delete");
    let page: Value = list(&c, &base, Some(&token), "")
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(page["total"], 0, "记录本身应删掉（文件跳过）");
}

// ---------- 6. 鉴权 ----------

#[tokio::test(flavor = "multi_thread")]
async fn media_endpoints_require_auth() {
    let tmp = tempfile::tempdir().unwrap();
    let base = spawn_server(tmp.path().join("config.toml").to_str().unwrap()).await;
    let c = reqwest::Client::new();
    let token = setup_installed(&c, &base, tmp.path()).await;

    let r = list(&c, &base, None, "").await;
    assert_eq!(r.status(), 401);
    assert_eq!(err_code(r).await, "unauthorized");

    let r = c
        .delete(format!("{base}/api/admin/media/1"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 401, "未登录删除 → 401（不泄露存在性）");

    // 已登录但 id 不存在 → 404
    let r = c
        .delete(format!("{base}/api/admin/media/9999"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 404);
    assert_eq!(err_code(r).await, "not_found");
}

// ---------- 7. 历史文件兜底：惰性扫描补建记录；非图片不入库 ----------

#[tokio::test(flavor = "multi_thread")]
async fn historical_files_are_scanned_into_list() {
    let tmp = tempfile::tempdir().unwrap();
    let base = spawn_server(tmp.path().join("config.toml").to_str().unwrap()).await;
    let c = reqwest::Client::new();
    let token = setup_installed(&c, &base, tmp.path()).await;

    // 模拟本次改动前的历史文件：直接写入 uploads/<yyyy>/<mm>/<hash16>.png（无 DB 记录）
    let dir = tmp.path().join("uploads").join("2025").join("12");
    std::fs::create_dir_all(&dir).unwrap();
    let hist = png_with_dims(8, 4, 20);
    let hist_name = "0123456789abcdef.png";
    std::fs::write(dir.join(hist_name), &hist).unwrap();
    // 同目录的非图片文件不应成为媒体条目
    std::fs::write(dir.join("notes.txt"), b"not an image").unwrap();

    let page: Value = list(&c, &base, Some(&token), "")
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(page["total"], 1, "历史图片应被兜底扫描到（非图片不入库）");
    let item = &page["items"][0];
    assert_eq!(item["url"], format!("/api/uploads/2025/12/{hist_name}"));
    assert_eq!(
        item["filename"], hist_name,
        "历史文件原始名缺失，回退存储文件名"
    );
    assert_eq!(item["size"], hist.len() as i64);
    assert_eq!(item["mime"], "image/png");
    assert_eq!(item["width"], 8);
    assert_eq!(item["height"], 4);
    assert!(item["id"].as_i64().unwrap() > 0);
    // created_at 取文件 mtime（RFC3339 文本）
    assert!(item["created_at"].as_str().unwrap().contains('T'));

    // 兜底文件同样可正常读取与删除
    let r = c
        .get(format!("{base}/api/uploads/2025/12/{hist_name}"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    let id = item["id"].as_i64().unwrap();
    let r = c
        .delete(format!("{base}/api/admin/media/{id}"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 204);
    assert!(!dir.join(hist_name).exists());

    // 再次列表：文件已删，不应被扫描复活；notes.txt 仍不入库
    let page: Value = list(&c, &base, Some(&token), "")
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(page["total"], 0);
}
