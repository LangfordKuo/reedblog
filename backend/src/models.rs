//! 契约中的数据形状（docs/api-contract.md「数据形状」一节）

use serde::{Deserialize, Serialize};

// ---------- 响应形状 ----------

#[derive(Debug, Clone, Serialize)]
pub struct SiteInfo {
    pub title: String,
    pub subtitle: String,
    pub installed: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct CategoryRef {
    pub id: i64,
    pub name: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct PostPublic {
    pub id: i64,
    pub title: String,
    pub slug: String,
    pub excerpt: String,
    pub category: Option<CategoryRef>,
    pub tags: Vec<CategoryRef>,
    pub published_at: String,
    pub comment_count: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct PostDetail {
    #[serde(flatten)]
    pub post: PostPublic,
    pub content_md: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct PostAdmin {
    pub id: i64,
    pub title: String,
    pub slug: String,
    pub content_md: String,
    pub excerpt: String,
    pub status: String,
    pub category_id: Option<i64>,
    pub category_name: Option<String>,
    pub tag_ids: Vec<i64>,
    pub published_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Category {
    pub id: i64,
    pub name: String,
    pub post_count: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Tag {
    pub id: i64,
    pub name: String,
    pub post_count: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct CommentPub {
    pub id: i64,
    pub author_name: String,
    pub content: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct CommentAdmin {
    pub id: i64,
    pub post_id: i64,
    pub post_title: String,
    pub author_name: String,
    pub email: Option<String>,
    pub content: String,
    pub status: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct AuthResult {
    pub token: String,
    pub username: String,
    pub expires_at: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ArchiveEntry {
    pub year: i64,
    pub month: i64,
    pub count: i64,
}

/// 分页响应统一形状
#[derive(Debug, Clone, Serialize)]
pub struct Page<T: Serialize> {
    pub items: Vec<T>,
    pub total: i64,
    pub page: i64,
    pub per_page: i64,
}

// ---------- 请求体 ----------

#[derive(Debug, Clone, Deserialize)]
pub struct MysqlInstallConfig {
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

#[derive(Debug, Clone, Deserialize)]
pub struct InstallSite {
    pub title: String,
    #[serde(default)]
    pub subtitle: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct InstallAdmin {
    pub username: String,
    pub password: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct InstallRequest {
    pub db_type: String,
    #[serde(default)]
    pub sqlite_path: Option<String>,
    #[serde(default)]
    pub mysql: Option<MysqlInstallConfig>,
    pub admin: InstallAdmin,
    pub site: InstallSite,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CreateCommentRequest {
    pub author_name: String,
    #[serde(default)]
    pub email: Option<String>,
    pub content: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PostBody {
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub slug: Option<String>,
    #[serde(default)]
    pub content_md: Option<String>,
    #[serde(default)]
    pub excerpt: Option<String>,
    /// 三态：字段缺失 = 不更新；显式 null = 清空分类；数字 = 设置分类
    #[serde(default, deserialize_with = "de_opt_opt")]
    pub category_id: Option<Option<i64>>,
    #[serde(default)]
    pub tag_ids: Option<Vec<i64>>,
    #[serde(default)]
    pub status: Option<String>,
}

fn de_opt_opt<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::Deserialize<'de>,
{
    Ok(Some(Option::deserialize(deserializer)?))
}

#[derive(Debug, Clone, Deserialize)]
pub struct NameBody {
    pub name: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CommentStatusBody {
    pub status: String,
}

// ---------- 查询参数 ----------

fn default_page() -> i64 {
    1
}

fn default_per_page() -> i64 {
    10
}

/// 归一化分页参数：page 最小 1；per_page 默认 10、上限 100。
pub fn normalize_paging(page: Option<i64>, per_page: Option<i64>) -> (i64, i64) {
    let page = page.unwrap_or_else(default_page).max(1);
    let per_page = per_page
        .unwrap_or_else(default_per_page)
        .clamp(1, 100);
    (page, per_page)
}

#[derive(Debug, Clone, Deserialize)]
pub struct PostsQuery {
    #[serde(default)]
    pub page: Option<i64>,
    #[serde(default)]
    pub per_page: Option<i64>,
    #[serde(default)]
    pub tag: Option<String>,
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default)]
    pub year: Option<i64>,
    #[serde(default)]
    pub month: Option<i64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AdminPostsQuery {
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub page: Option<i64>,
    #[serde(default)]
    pub per_page: Option<i64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AdminCommentsQuery {
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub post_id: Option<i64>,
    #[serde(default)]
    pub page: Option<i64>,
    #[serde(default)]
    pub per_page: Option<i64>,
}
