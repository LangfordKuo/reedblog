//! 站点设置（契约「站点设置」条款）：settings 表 key-value 存储的读写、默认值回退与校验。
//!
//! 硬性约束：
//! - SQLite/MySQL 共用一份 SQL（sqlx Any 驱动，`?` 占位符），无单方言语法：
//!   upsert 用「先 UPDATE，rows_affected=0 再 INSERT」实现（不依赖 ON CONFLICT /
//!   ON DUPLICATE KEY 等方言语法）；
//! - 进程内实时生效：读取路径每次请求查库，修改后无需重启；
//! - 时间戳沿用全库 RFC3339 UTC 文本惯例（updated_at）；
//! - 旧库升级（表存在但缺行）时按键回退默认值：title/subtitle 回退 config.toml [site]
//!   （即 Runtime 缓存值），base_url 回退 config.toml [server] base_url，per_page 回退 10。

use sqlx::{AnyConnection, AnyPool, Row};

use crate::error::{ApiError, ApiResult};
use crate::state::{now_rfc3339, AppState};

/// settings 表键名（契约「站点设置」字段清单）
pub const KEY_TITLE: &str = "site_title";
pub const KEY_SUBTITLE: &str = "site_subtitle";
pub const KEY_DESCRIPTION: &str = "site_description";
pub const KEY_ICP: &str = "icp_number";
pub const KEY_FOOTER: &str = "footer_text";
pub const KEY_PER_PAGE: &str = "per_page";
pub const KEY_BASE_URL: &str = "base_url";

/// per_page 默认值（与契约总则分页默认一致）
pub const DEFAULT_PER_PAGE: i64 = 10;

/// 站点设置全集（base_url 为敏感字段，公开接口序列化时单独裁剪）
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SiteSettings {
    pub title: String,
    pub subtitle: String,
    pub description: String,
    pub icp_number: String,
    pub footer_text: String,
    pub per_page: i64,
    pub base_url: String,
}

/// upsert（连接版）：先 UPDATE，rows_affected=0 再 INSERT。
/// 双方言共用一份 SQL，不用 ON CONFLICT / ON DUPLICATE KEY 等单方言语法
async fn upsert_conn(conn: &mut AnyConnection, name: &str, value: &str) -> Result<(), sqlx::Error> {
    let now = now_rfc3339();
    let updated = sqlx::query("UPDATE settings SET value = ?, updated_at = ? WHERE name = ?")
        .bind(value)
        .bind(&now)
        .bind(name)
        .execute(&mut *conn)
        .await?
        .rows_affected();
    if updated == 0 {
        sqlx::query("INSERT INTO settings (name, value, updated_at) VALUES (?, ?, ?)")
            .bind(name)
            .bind(value)
            .bind(&now)
            .execute(&mut *conn)
            .await?;
    }
    Ok(())
}

/// upsert（连接池版），SQL 与连接版一致
async fn upsert_pool(pool: &AnyPool, name: &str, value: &str) -> Result<(), sqlx::Error> {
    let now = now_rfc3339();
    let updated = sqlx::query("UPDATE settings SET value = ?, updated_at = ? WHERE name = ?")
        .bind(value)
        .bind(&now)
        .bind(name)
        .execute(pool)
        .await?
        .rows_affected();
    if updated == 0 {
        sqlx::query("INSERT INTO settings (name, value, updated_at) VALUES (?, ?, ?)")
            .bind(name)
            .bind(value)
            .bind(&now)
            .execute(pool)
            .await?;
    }
    Ok(())
}

/// 把一整份设置写入（安装默认值注入路径；逐键 upsert）
pub async fn save_on_conn(conn: &mut AnyConnection, s: &SiteSettings) -> Result<(), sqlx::Error> {
    for (name, value) in to_pairs(s) {
        upsert_conn(conn, name, &value).await?;
    }
    Ok(())
}

/// 把一整份设置写入连接池（管理端 PUT 路径）
pub async fn save(pool: &AnyPool, s: &SiteSettings) -> Result<(), sqlx::Error> {
    for (name, value) in to_pairs(s) {
        upsert_pool(pool, name, &value).await?;
    }
    Ok(())
}

fn to_pairs(s: &SiteSettings) -> [(&'static str, String); 7] {
    [
        (KEY_TITLE, s.title.clone()),
        (KEY_SUBTITLE, s.subtitle.clone()),
        (KEY_DESCRIPTION, s.description.clone()),
        (KEY_ICP, s.icp_number.clone()),
        (KEY_FOOTER, s.footer_text.clone()),
        (KEY_PER_PAGE, s.per_page.to_string()),
        (KEY_BASE_URL, s.base_url.clone()),
    ]
}

/// 读取设置：查库后按键补缺（旧库升级/安装注入失败时的默认值回退，见模块头注释）。
/// per_page 解析失败或越界时钳回 1~100（非法值兜底 DEFAULT_PER_PAGE）。
pub async fn load(pool: &AnyPool, state: &AppState) -> ApiResult<SiteSettings> {
    let rt = state.runtime().await;
    let rows = sqlx::query("SELECT name, value FROM settings")
        .fetch_all(pool)
        .await?;

    let get = |key: &str| -> Option<String> {
        rows.iter()
            .find(|r| r.get::<String, _>("name") == key)
            .map(|r| r.get::<String, _>("value"))
    };

    let per_page = get(KEY_PER_PAGE)
        .and_then(|v| v.trim().parse::<i64>().ok())
        .map(|v| v.clamp(1, 100))
        .unwrap_or(DEFAULT_PER_PAGE);

    Ok(SiteSettings {
        title: get(KEY_TITLE)
            .filter(|v| !v.trim().is_empty())
            .unwrap_or_else(|| rt.site_title.clone()),
        subtitle: get(KEY_SUBTITLE).unwrap_or_else(|| rt.site_subtitle.clone()),
        description: get(KEY_DESCRIPTION).unwrap_or_default(),
        icp_number: get(KEY_ICP).unwrap_or_default(),
        footer_text: get(KEY_FOOTER).unwrap_or_default(),
        per_page,
        base_url: get(KEY_BASE_URL).unwrap_or_else(|| state.configured_base_url()),
    })
}

/// 安装时写入默认值：title/subtitle 取安装请求，base_url 初始值取 config.toml
/// [server] base_url，per_page=10，其余为空串（契约「站点设置-默认值」条款）
pub fn install_defaults(title: &str, subtitle: &str, base_url: &str) -> SiteSettings {
    SiteSettings {
        title: title.trim().to_string(),
        subtitle: subtitle.trim().to_string(),
        description: String::new(),
        icp_number: String::new(),
        footer_text: String::new(),
        per_page: DEFAULT_PER_PAGE,
        base_url: base_url.trim().trim_end_matches('/').to_string(),
    }
}

/// PUT 前校验（契约「站点设置」校验条款）；失败 → 422 validation_error
pub fn validate(s: &SiteSettings) -> ApiResult<()> {
    if s.title.trim().is_empty() {
        return Err(ApiError::validation("站点名称不能为空"));
    }
    fn len_ok(v: &str, max: usize, label: &str) -> ApiResult<()> {
        if v.chars().count() > max {
            return Err(ApiError::validation(format!(
                "{label} 过长（上限 {max} 字符）"
            )));
        }
        Ok(())
    }
    len_ok(&s.title, 255, "站点名称")?;
    len_ok(&s.subtitle, 255, "副标题")?;
    len_ok(&s.description, 1000, "站点描述")?;
    len_ok(&s.icp_number, 100, "ICP 备案号")?;
    len_ok(&s.footer_text, 1000, "页脚文字")?;
    len_ok(&s.base_url, 500, "base_url")?;

    if !(1..=100).contains(&s.per_page) {
        return Err(ApiError::validation(
            "每页文章数 per_page 必须是 1~100 的整数",
        ));
    }
    if !s.base_url.is_empty() {
        match url::Url::parse(&s.base_url) {
            Ok(u) if (u.scheme() == "http" || u.scheme() == "https") && u.host().is_some() => {}
            _ => {
                return Err(ApiError::validation(
                    "base_url 必须是合法的 http/https 绝对 URL（如 https://blog.example.com）",
                ))
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> SiteSettings {
        SiteSettings {
            title: "测试博客".to_string(),
            subtitle: String::new(),
            description: String::new(),
            icp_number: String::new(),
            footer_text: String::new(),
            per_page: DEFAULT_PER_PAGE,
            base_url: String::new(),
        }
    }

    #[test]
    fn accepts_valid_settings() {
        assert!(validate(&base()).is_ok());
        let mut s = base();
        s.base_url = "https://blog.example.com".to_string();
        s.per_page = 100;
        assert!(validate(&s).is_ok());
        s.per_page = 1;
        s.base_url = "http://localhost:3000".to_string();
        assert!(validate(&s).is_ok());
    }

    #[test]
    fn rejects_invalid_settings() {
        let mut s = base();
        s.title = "   ".to_string();
        assert!(validate(&s).is_err(), "空标题应被拒绝");

        let mut s = base();
        s.per_page = 0;
        assert!(validate(&s).is_err());
        s.per_page = 101;
        assert!(validate(&s).is_err());

        for bad in ["not a url", "ftp://x.dev", "blog.example.com", "https://"] {
            let mut s = base();
            s.base_url = bad.to_string();
            assert!(validate(&s).is_err(), "非法 base_url 应被拒绝: {bad}");
        }

        let mut s = base();
        s.description = "长".repeat(1001);
        assert!(validate(&s).is_err(), "超长描述应被拒绝");
    }

    #[test]
    fn install_defaults_follow_contract() {
        let s = install_defaults(" 我的博客 ", " 口号 ", "https://x.dev/");
        assert_eq!(s.title, "我的博客");
        assert_eq!(s.subtitle, "口号");
        assert_eq!(s.base_url, "https://x.dev"); // 去尾 /
        assert_eq!(s.per_page, DEFAULT_PER_PAGE);
        assert_eq!(s.description, "");
        assert_eq!(s.icp_number, "");
        assert_eq!(s.footer_text, "");
    }
}
