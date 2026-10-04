//! 启动语义（2026-10-04）：config.toml 标记已安装（jwt_secret 非空）但数据库连接/迁移
//! 失败时，后端必须非零退出、绝不回退为未安装态——否则管理员可能误走安装向导覆盖已有数据。
//! 用真实二进制（CARGO_BIN_EXE_reedblog-backend）起进程验证：
//! - case A：已安装库的迁移校验和被篡改（模拟迁移文件被改动/库与二进制不匹配）
//!   → 拒绝启动：非零退出 + 醒目错误日志（含原始错误串与处置建议）
//! - case B：jwt_secret 为空（未安装态）→ 照旧正常启动，/api/site/settings 返回 503

use serde_json::{json, Value};
use sqlx::Connection;
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;
use tokio::io::AsyncReadExt;
use tokio::process::{Child, Command};

/// 真实后端二进制（cargo 为集成测试注入的路径）
fn backend_bin() -> &'static str {
    env!("CARGO_BIN_EXE_reedblog-backend")
}

/// 进程内起一个真实服务（跑一次真实 /api/install 用；与 integration.rs 同款辅助）。
/// 配置文件不存在时预写 [plugins]/[themes] dir 指向临时目录，避免污染仓库工作目录。
async fn spawn_inprocess_server(config_path: &str) -> String {
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

/// 用真实二进制起一个后端进程（工作目录 = 临时目录，运行时文件不落到仓库里）
fn spawn_backend(config_path: &Path, cwd: &Path) -> Child {
    Command::new(backend_bin())
        .env("REEDBLOG_CONFIG", config_path)
        .current_dir(cwd)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .expect("无法启动 reedblog-backend 二进制")
}

/// 读取子进程 stderr（进程已退出或已 kill 后调用；输出量很小，不会写满管道）
async fn read_stderr(child: &mut Child) -> String {
    let Some(mut err) = child.stderr.take() else {
        return String::new();
    };
    let mut bytes = Vec::new();
    let _ = err.read_to_end(&mut bytes).await;
    String::from_utf8_lossy(&bytes).into_owned()
}

/// 取一个空闲端口（仅用于让未安装态的二进制有确定的监听端口）
fn free_port() -> u16 {
    let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    l.local_addr().unwrap().port()
}

// ---------- case A：已安装配置 + 迁移校验和被篡改 → 拒绝启动 ----------

#[tokio::test(flavor = "multi_thread")]
async fn installed_config_with_broken_migrations_refuses_to_start() {
    let tmp = tempfile::tempdir().unwrap();
    let cfg_path = tmp.path().join("config.toml");
    let db_path = tmp.path().join("reedblog.db");
    let c = reqwest::Client::new();

    // ---- 1. 正常安装路径：真实 /api/install 写 config.toml（jwt_secret 非空）并建库 ----
    let base = spawn_inprocess_server(cfg_path.to_str().unwrap()).await;
    let r = c
        .post(format!("{base}/api/install"))
        .json(&json!({
            "db_type": "sqlite",
            "sqlite_path": db_path.to_str().unwrap(),
            "admin": {"username": "admin", "password": "secret123"},
            "site": {"title": "启动语义测试", "subtitle": "副标题"}
        }))
        .send()
        .await
        .unwrap();
    let status = r.status();
    assert_eq!(status, 201, "install 应成功: {}", r.text().await.unwrap());
    let cfg_text = std::fs::read_to_string(&cfg_path).unwrap();
    assert!(
        cfg_text.contains("jwt_secret"),
        "config.toml 应有安装写入的 jwt_secret: {cfg_text}"
    );

    // ---- 2. 篡改任意一条已应用迁移的校验和（模拟迁移文件被改动/库与二进制不匹配）----
    let opts = sqlx::sqlite::SqliteConnectOptions::new().filename(&db_path);
    let mut conn = sqlx::SqliteConnection::connect_with(&opts).await.unwrap();
    let affected = sqlx::query(
        "UPDATE _sqlx_migrations SET checksum = randomblob(48) \
         WHERE version = (SELECT MIN(version) FROM _sqlx_migrations)",
    )
    .execute(&mut conn)
    .await
    .unwrap()
    .rows_affected();
    assert_eq!(affected, 1, "应篡改到一条迁移记录");
    drop(conn);

    // ---- 3. 用真实二进制 + 该配置启动：必须拒绝启动、非零退出 ----
    let mut child = spawn_backend(&cfg_path, tmp.path());
    let status = match tokio::time::timeout(Duration::from_secs(30), child.wait()).await {
        Ok(s) => s.unwrap(),
        Err(_) => {
            child.kill().await.ok();
            let stderr = read_stderr(&mut child).await;
            panic!("后端未拒绝启动（超时仍在运行）:\nstderr:\n{stderr}");
        }
    };
    let stderr = read_stderr(&mut child).await;

    assert!(
        !status.success(),
        "必须非零退出，实际 {status:?}:\nstderr:\n{stderr}"
    );
    assert_eq!(
        status.code(),
        Some(1),
        "退出码应为 1（main.rs 的启动失败路径）:\nstderr:\n{stderr}"
    );
    // 新文案：结论 + 原始错误串 + 可操作提示
    assert!(
        stderr.contains("拒绝启动"),
        "stderr 应含「拒绝启动」:\n{stderr}"
    );
    assert!(
        stderr.contains("was previously applied but has been modified"),
        "stderr 应保留原始错误串:\n{stderr}"
    );
    assert!(
        stderr.contains("备份数据库与上传目录"),
        "stderr 应含处置建议:\n{stderr}"
    );
    // 绝不回退未安装态
    assert!(
        !stderr.contains("以未安装状态启动"),
        "不应再出现「以未安装状态启动」降级文案:\n{stderr}"
    );
}

// ---------- case B：未安装态（jwt_secret 为空）→ 照旧起服务，接口 503 ----------

#[tokio::test(flavor = "multi_thread")]
async fn uninstalled_startup_still_serves_install_wizard() {
    let tmp = tempfile::tempdir().unwrap();
    let port = free_port();
    let cfg_path = tmp.path().join("config.toml");
    // 配置文件存在只为固定端口与隔离目录；无 [auth] jwt_secret → 未安装态
    std::fs::write(
        &cfg_path,
        format!("[server]\nhost = \"127.0.0.1\"\nport = {port}\n"),
    )
    .unwrap();

    let mut child = spawn_backend(&cfg_path, tmp.path());
    let base = format!("http://127.0.0.1:{port}");
    let c = reqwest::Client::builder()
        .timeout(Duration::from_millis(800))
        .build()
        .unwrap();

    // 轮询等端口就绪；超时判失败并打印子进程 stderr
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    let resp = loop {
        match c.get(format!("{base}/api/site/settings")).send().await {
            Ok(r) => break r,
            Err(e) => {
                if tokio::time::Instant::now() >= deadline {
                    child.kill().await.ok();
                    let stderr = read_stderr(&mut child).await;
                    panic!("后端未在 20s 内就绪: {e}\nstderr:\n{stderr}");
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }
    };

    // 公开接口未安装 → 503 not_installed
    assert_eq!(resp.status(), 503);
    let v: Value = resp.json().await.unwrap();
    assert_eq!(v["error"]["code"], "not_installed");

    // 安装向导入口照旧可用
    let v: Value = c
        .get(format!("{base}/api/install/status"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(v, json!({"installed": false}));

    child.kill().await.ok();
    let _ = child.wait().await;
}
