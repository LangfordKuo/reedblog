//! 应用状态：安装前为空壳，安装完成（或启动时读到已安装配置）后持有连接池与 JWT secret。
//! 另含 SQLite/MySQL 通用的数据库辅助函数。

use chrono::Utc;
use sqlx::any::{install_default_drivers, AnyConnectOptions, AnyPoolOptions};
use sqlx::{AnyConnection, AnyPool, Connection};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;

use crate::config::{
    default_max_size_mb, default_plugins_dir, default_themes_dir, default_uploads_dir, Config,
};
use crate::error::ApiResult;
use crate::plugins::PluginHost;

/// 运行期状态（进程内可变）
#[derive(Debug, Clone)]
pub struct Runtime {
    pub installed: bool,
    pub db_type: String,
    pub pool: Option<AnyPool>,
    pub jwt_secret: Option<String>,
    pub site_title: String,
    pub site_subtitle: String,
}

impl Default for Runtime {
    fn default() -> Self {
        Self {
            installed: false,
            db_type: String::new(),
            pool: None,
            jwt_secret: None,
            site_title: "reedblog".to_string(),
            site_subtitle: String::new(),
        }
    }
}

struct Inner {
    config_path: String,
    runtime: RwLock<Runtime>,
    /// 插件宿主（内存注册表 + 插件目录），Clone 共享
    plugins: PluginHost,
    /// 主题存储根目录（active 的权威来源是 config.toml，目录本身启动时解析一次）
    themes_dir: PathBuf,
    /// 图片上传存储根目录（config.toml [uploads] dir，启动时解析一次）
    uploads_dir: PathBuf,
}

#[derive(Clone)]
pub struct AppState {
    inner: Arc<Inner>,
}

impl AppState {
    pub fn new(config_path: impl Into<String>) -> Self {
        // sqlx 0.8 的 Any 驱动要求显式注册底层驱动（幂等，可重复调用）
        install_default_drivers();
        let config_path = config_path.into();
        // 插件/主题目录来自 config.toml（[plugins] dir / [themes] dir，缺省 plugins、themes）
        let cfg = Config::load(Path::new(&config_path));
        let plugins_dir = PathBuf::from(
            cfg.as_ref()
                .map(|c| c.plugins.dir.clone())
                .unwrap_or_else(default_plugins_dir),
        );
        let themes_dir = PathBuf::from(
            cfg.as_ref()
                .map(|c| c.themes.dir.clone())
                .unwrap_or_else(default_themes_dir),
        );
        let uploads_dir = PathBuf::from(
            cfg.as_ref()
                .map(|c| c.uploads.dir.clone())
                .unwrap_or_else(default_uploads_dir),
        );
        Self {
            inner: Arc::new(Inner {
                config_path,
                runtime: RwLock::new(Runtime::default()),
                plugins: PluginHost::new(plugins_dir),
                themes_dir,
                uploads_dir,
            }),
        }
    }

    pub fn config_path(&self) -> &str {
        &self.inner.config_path
    }

    pub fn plugins(&self) -> &PluginHost {
        &self.inner.plugins
    }

    pub fn themes_dir(&self) -> &Path {
        &self.inner.themes_dir
    }

    pub fn uploads_dir(&self) -> &Path {
        &self.inner.uploads_dir
    }

    /// config.toml 中当前激活主题 slug（读不到配置时回退 default）
    pub fn active_theme_slug(&self) -> String {
        Config::load(Path::new(&self.inner.config_path))
            .map(|c| c.themes.active)
            .unwrap_or_else(crate::config::default_active_theme)
    }

    /// 上传图片大小上限（字节）：config.toml [uploads] max_size_mb，
    /// 读不到配置时回退默认 10MB（每次请求读取，改配置无需重启）
    pub fn uploads_max_size_bytes(&self) -> usize {
        let mb = Config::load(Path::new(&self.inner.config_path))
            .map(|c| c.uploads.max_size_mb)
            .unwrap_or_else(default_max_size_mb);
        (mb as usize).saturating_mul(1024 * 1024)
    }

    /// config.toml [server] base_url（未配置/读不到时为空串；feed/sitemap 用）
    pub fn configured_base_url(&self) -> String {
        Config::load(Path::new(&self.inner.config_path))
            .map(|c| c.server.base_url)
            .unwrap_or_default()
    }

    pub async fn is_installed(&self) -> bool {
        self.inner.runtime.read().await.installed
    }

    pub async fn runtime(&self) -> Runtime {
        self.inner.runtime.read().await.clone()
    }

    /// 切换到已安装状态（安装向导末尾 / 启动恢复时调用）
    pub async fn activate(
        &self,
        db_type: String,
        pool: AnyPool,
        jwt_secret: String,
        site_title: String,
        site_subtitle: String,
    ) {
        let mut rt = self.inner.runtime.write().await;
        rt.installed = true;
        rt.db_type = db_type;
        rt.pool = Some(pool);
        rt.jwt_secret = Some(jwt_secret);
        rt.site_title = site_title;
        rt.site_subtitle = site_subtitle;
    }
}

// ---------- 数据库辅助 ----------

/// 当前时间的 RFC3339 UTC 字符串（秒精度，如 2026-10-03T12:00:00Z）。
/// 全库时间戳统一此格式：字典序即时间序，SQLite/MySQL 查询逻辑可共用。
pub fn now_rfc3339() -> String {
    Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

/// 连接数据库并跑迁移（sqlite / mysql 均走 sqlx Any 驱动）。
/// url 由 config.db_url() 生成。SQLite 侧 from_str 是剥 "sqlite://" 前缀的纯字符串解析，
/// Windows 盘符绝对路径（sqlite://D:/x/y.db）可正确往返（空格等会被百分号解码）。
pub async fn connect_pool(db_type: &str, url: &str) -> Result<AnyPool, sqlx::Error> {
    install_default_drivers();
    let opts: AnyConnectOptions = url.parse()?;
    // 先用单连接快速验证连通性并建表，再建正式池
    let mut probe = AnyConnection::connect_with(&opts).await?;
    run_migrations(db_type, &mut probe).await?;
    drop(probe);

    let pool = AnyPoolOptions::new()
        .max_connections(if db_type == "sqlite" { 4 } else { 10 })
        .acquire_timeout(Duration::from_secs(10))
        .connect_with(opts)
        .await?;
    // 历史数据修复：旧版推导算法（只压平空白、未剥 Markdown）写入的脏 excerpt 按现行规则重写（幂等）
    crate::handlers::helpers::repair_legacy_excerpts(&pool).await?;
    Ok(pool)
}

/// 按 db_type 跑对应的内嵌迁移（migrations/sqlite 或 migrations/mysql）
pub async fn run_migrations(
    db_type: &str,
    conn: &mut AnyConnection,
) -> Result<(), sqlx::Error> {
    let migrator = match db_type {
        "sqlite" => sqlx::migrate!("migrations/sqlite"),
        "mysql" => sqlx::migrate!("migrations/mysql"),
        other => {
            return Err(sqlx::Error::Configuration(
                format!("不支持的 db_type: {other}").into(),
            ))
        }
    };
    // 用 run_direct 而非 run：run 的 Acquire 泛型存在高阶生命周期问题
    // （"implementation of Acquire is not general enough"），会导致包含它的
    // async fn 生成器无法证明 Send，进而使 axum Handler 约束失败。
    migrator.run_direct(conn).await.map_err(sqlx::Error::from)
}

/// 在同一连接上取最近一次 INSERT 的自增 id（SQLite/MySQL 方言不同）
pub async fn last_insert_id(conn: &mut AnyConnection, db_type: &str) -> Result<i64, sqlx::Error> {
    let sql = match db_type {
        "mysql" => "SELECT CAST(LAST_INSERT_ID() AS SIGNED)",
        _ => "SELECT last_insert_rowid()",
    };
    sqlx::query_scalar::<_, i64>(sql).fetch_one(conn).await
}

/// 取连接池；未安装/池不存在时返回 not_installed（正常情况下被中间件挡住，不会走到）
pub async fn require_pool(state: &AppState) -> ApiResult<(AnyPool, String)> {
    let rt = state.runtime().await;
    match rt.pool {
        Some(pool) if rt.installed => Ok((pool, rt.db_type.clone())),
        _ => Err(crate::error::ApiError::not_installed()),
    }
}

