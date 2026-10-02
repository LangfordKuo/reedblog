use serde::{Deserialize, Serialize};
use std::path::Path;

/// config.toml 的完整结构（安装向导写入，进程启动时读取）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub server: ServerConfig,
    // database/site/auth 允许缺省：安装前可只写 [plugins]/[themes] 等段预配置目录；
    // 「已安装」仍由非空 jwt_secret + 可连数据库判定，缺省段落不影响该判定
    #[serde(default)]
    pub database: DatabaseConfig,
    #[serde(default)]
    pub site: SiteConfig,
    #[serde(default)]
    pub auth: AuthConfig,
    #[serde(default)]
    pub cors: CorsConfig,
    #[serde(default)]
    pub plugins: PluginsConfig,
    #[serde(default)]
    pub themes: ThemesConfig,
}

/// [plugins]：插件存储根目录（相对后端运行目录或绝对路径）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginsConfig {
    #[serde(default = "default_plugins_dir")]
    pub dir: String,
}

impl Default for PluginsConfig {
    fn default() -> Self {
        Self {
            dir: default_plugins_dir(),
        }
    }
}

pub fn default_plugins_dir() -> String {
    "plugins".to_string()
}

/// [themes]：主题存储根目录 + 当前激活主题 slug（active 权威来源）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThemesConfig {
    #[serde(default = "default_themes_dir")]
    pub dir: String,
    #[serde(default = "default_active_theme")]
    pub active: String,
}

impl Default for ThemesConfig {
    fn default() -> Self {
        Self {
            dir: default_themes_dir(),
            active: default_active_theme(),
        }
    }
}

pub fn default_themes_dir() -> String {
    "themes".to_string()
}

pub fn default_active_theme() -> String {
    "default".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    #[serde(default = "default_host")]
    pub host: String,
    #[serde(default = "default_port")]
    pub port: u16,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            host: default_host(),
            port: default_port(),
        }
    }
}

fn default_host() -> String {
    "127.0.0.1".to_string()
}

fn default_port() -> u16 {
    3000
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatabaseConfig {
    /// "sqlite" | "mysql"
    #[serde(default)]
    pub db_type: String,
    #[serde(default = "default_sqlite_path")]
    pub sqlite_path: String,
    #[serde(default)]
    pub mysql: Option<MysqlConfig>,
}

impl Default for DatabaseConfig {
    fn default() -> Self {
        Self {
            db_type: String::new(),
            sqlite_path: default_sqlite_path(),
            mysql: None,
        }
    }
}

fn default_sqlite_path() -> String {
    "reedblog.db".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MysqlConfig {
    pub host: String,
    #[serde(default = "default_mysql_port")]
    pub port: u16,
    pub username: String,
    #[serde(default)]
    pub password: String,
    pub database: String,
}

fn default_mysql_port() -> u16 {
    3306
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SiteConfig {
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub subtitle: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AuthConfig {
    #[serde(default)]
    pub jwt_secret: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CorsConfig {
    #[serde(default = "default_origins")]
    pub allowed_origins: Vec<String>,
}

impl Default for CorsConfig {
    fn default() -> Self {
        Self {
            allowed_origins: default_origins(),
        }
    }
}

pub fn default_origins() -> Vec<String> {
    vec!["http://localhost:5173".to_string()]
}

impl Config {
    /// 从磁盘加载；文件不存在或解析失败返回 None（视为未安装）
    pub fn load(path: &Path) -> Option<Config> {
        let text = std::fs::read_to_string(path).ok()?;
        match toml::from_str::<Config>(&text) {
            Ok(c) => Some(c),
            Err(e) => {
                eprintln!("[reedblog] 解析 config.toml 失败: {e}");
                None
            }
        }
    }

    /// 根据配置构造数据库连接 URL（sqlite / mysql）。
    /// sqlite 路径经 encode_sqlite_path 百分号编码：AnyConnectOptions 会经 url::Url
    /// 往返，Windows 盘符冒号若不编码为 %3A 会被当作空端口丢弃（SqliteConnectOptions
    /// 解析时会百分号解码还原为原始路径）。
    pub fn db_url(&self) -> Option<String> {
        match self.database.db_type.as_str() {
            "sqlite" => Some(format!(
                "sqlite://{}?mode=rwc",
                encode_sqlite_path(&self.database.sqlite_path)
            )),
            "mysql" => self.database.mysql.as_ref().map(|m| {
                format!(
                    "mysql://{}:{}@{}:{}/{}",
                    urlencoding::encode(&m.username),
                    urlencoding::encode(&m.password),
                    m.host,
                    m.port,
                    urlencoding::encode(&m.database)
                )
            }),
            _ => None,
        }
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }
        let text = toml::to_string_pretty(self)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
        std::fs::write(path, text)
    }
}

/// 把本地文件路径转成可安全经 url::Url 往返的 sqlite:// 路径段：
/// 1) 反斜杠 → 正斜杠；2) 百分号编码 %、#、?、空格（解析端会解码还原）；
/// 3) Windows 盘符冒号编码为 %3A（否则 "C:" 在 authority 位置被 url 当作空端口丢冒号）。
fn encode_sqlite_path(path: &str) -> String {
    let normalized = path.replace('\\', "/");
    let mut out = String::with_capacity(normalized.len() + 8);
    for ch in normalized.chars() {
        match ch {
            '%' => out.push_str("%25"),
            '#' => out.push_str("%23"),
            '?' => out.push_str("%3F"),
            ' ' => out.push_str("%20"),
            _ => out.push(ch),
        }
    }
    let b = out.as_bytes();
    if b.len() >= 2 && b[1] == b':' && b[0].is_ascii_alphabetic() {
        out.replace_range(1..2, "%3A");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg_with_sqlite_path(path: &str) -> Config {
        Config {
            server: ServerConfig::default(),
            database: DatabaseConfig {
                db_type: "sqlite".to_string(),
                sqlite_path: path.to_string(),
                mysql: None,
            },
            site: SiteConfig {
                title: String::new(),
                subtitle: None,
            },
            auth: AuthConfig {
                jwt_secret: String::new(),
            },
            cors: CorsConfig::default(),
            plugins: PluginsConfig::default(),
            themes: ThemesConfig::default(),
        }
    }

    /// db_url 经 AnyConnectOptions(Url) 往返后，SQLite 解析出的文件名必须与原路径一致
    #[test]
    fn sqlite_db_url_roundtrip() {
        for path in [
            "reedblog.db",
            "D:\\tmp\\x y\\reedblog.db",
            "C:/Users/test/reedblog.db",
            "/var/lib/reedblog/reedblog.db",
        ] {
            let url = cfg_with_sqlite_path(path).db_url().unwrap();
            let any_opts: sqlx::any::AnyConnectOptions = url
                .parse()
                .unwrap_or_else(|e| panic!("{url} 无法解析为 AnyConnectOptions: {e}"));
            let sqlite_opts: sqlx::sqlite::SqliteConnectOptions = any_opts
                .database_url
                .as_str()
                .parse()
                .unwrap_or_else(|e| panic!("{url} 往返后无法解析为 SqliteConnectOptions: {e}"));
            let got = sqlite_opts.get_filename().to_string_lossy().replace('\\', "/");
            assert_eq!(got, path.replace('\\', "/"), "路径 {path} 往返后不一致 (url={url})");
        }
    }
}
