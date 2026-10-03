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
    /// 是否置顶（契约「文章置顶与定时发布」条款）；前台列表卡显示「置顶」徽章
    pub is_sticky: bool,
    /// 浏览量（契约「浏览量与点赞」条款）；列表/详情卡片眼睛图标显示
    pub view_count: i64,
    /// 点赞总数（post_likes 子查询计数；契约「浏览量与点赞」条款）
    pub likes: i64,
}

/// 上一篇/下一篇导航项（契约「文章上一篇/下一篇」条款，2026-10-04 新增）：
/// 只带标题与 slug，不含正文；某方向无相邻文章时详情响应中为 null
#[derive(Debug, Clone, Serialize)]
pub struct PostNeighbor {
    pub title: String,
    pub slug: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct PostDetail {
    #[serde(flatten)]
    pub post: PostPublic,
    /// 经 post.before_render 钩子链改写后的 Markdown（未启用插件时即库中原文）
    pub content_md: String,
    /// 渲染管线产物：before_render 改写 content_md → Markdown 渲染 → after_render 改写。
    /// 扩展契约新增字段（核心契约 PostDetail 的超集，前端可继续只用 content_md）
    pub content_html: String,
    /// 发布时间更早的相邻文章（纯时间序，不受置顶影响）；无则 null
    pub prev_post: Option<PostNeighbor>,
    /// 发布时间更晚的相邻文章；无则 null
    pub next_post: Option<PostNeighbor>,
}

/// 搜索结果单条（契约「全文搜索」条款）：PostPublic 全字段 + snippet。
/// snippet 为纯文本上下文片段（不含任何 HTML/Markdown markup），命中高亮由前端实现。
#[derive(Debug, Clone, Serialize)]
pub struct SearchResult {
    #[serde(flatten)]
    pub post: PostPublic,
    pub snippet: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct PostAdmin {
    pub id: i64,
    pub title: String,
    pub slug: String,
    pub content_md: String,
    pub excerpt: String,
    /// "draft" | "published" | "scheduled"（scheduled 为定时发布，契约 2026-10-03 新增）
    pub status: String,
    pub category_id: Option<i64>,
    pub category_name: Option<String>,
    pub tag_ids: Vec<i64>,
    /// status=scheduled 时即计划发布时间（RFC3339 UTC）
    pub published_at: Option<String>,
    /// 是否置顶（契约「文章置顶与定时发布」条款）
    pub is_sticky: bool,
    /// 浏览量（后台只读展示；契约「浏览量与点赞」条款）
    pub view_count: i64,
    /// 点赞总数（后台只读展示，不做管理点赞；契约「浏览量与点赞」条款）
    pub likes: i64,
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
    /// 所属顶级楼层 id（契约「评论回复」条款）；顶级评论本身为 null
    pub parent_id: Option<i64>,
    /// 被回复的中间楼层 id（仅「回复的回复」非 null；两级归一化后存储）
    pub reply_to_id: Option<i64>,
    /// 被回复人作者名（JOIN 冗余；reply_to_id 为 null 时同为 null）
    pub reply_to_name: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CommentAdmin {
    pub id: i64,
    /// 目标 id（契约 2026-10-03 页面功能扩展）：target_type='post' 时为文章 id，
    /// 'page' 时为页面 id（复用 comments.post_id 列，对现有契约破坏最小）
    pub post_id: i64,
    /// 目标标题（文章标题或页面标题）
    pub post_title: String,
    pub author_name: String,
    pub email: Option<String>,
    pub content: String,
    pub status: String,
    pub created_at: String,
    /// 评论来源："post"（文章）| "page"（页面留言）
    pub target_type: String,
    /// 所属顶级楼层 id（契约「评论回复」条款）；顶级评论本身为 null
    pub parent_id: Option<i64>,
    /// 被回复的中间楼层 id（仅「回复的回复」非 null）
    pub reply_to_id: Option<i64>,
    /// 被回复人作者名（JOIN 冗余）
    pub reply_to_name: Option<String>,
    /// 直接子回复条数（两级存储下即整线程楼层数；子回复恒 0）。
    /// 供后台提示「删除将连带删除 N 条回复」
    pub reply_count: i64,
}

// ---------- 页面（契约「页面」条款，2026-10-03 新增） ----------

/// GET /api/pages 单条（公开列表摘要，仅 enabled）
#[derive(Debug, Clone, Serialize)]
pub struct PageSummary {
    pub id: i64,
    pub title: String,
    pub slug: String,
    /// "custom" | "message_board" | "links"（前端据此决定渲染留言表单/链接列表）
    pub kind: String,
    pub sort_order: i64,
}

/// 友情链接单条（kind=links 页面附带；公开与管理端同形状）
#[derive(Debug, Clone, Serialize)]
pub struct PageLink {
    pub id: i64,
    pub name: String,
    pub url: String,
    pub description: String,
    pub sort_order: i64,
}

/// GET /api/pages/:slug 响应（content_html 为钩子管线实时渲染产物；
/// links 仅 kind=links 时非空，其余 kind 恒为 []）
#[derive(Debug, Clone, Serialize)]
pub struct PageDetail {
    pub id: i64,
    pub title: String,
    pub slug: String,
    pub kind: String,
    pub content_html: String,
    pub sort_order: i64,
    pub updated_at: String,
    pub links: Vec<PageLink>,
}

/// 管理端页面形状（GET/POST/PUT/PATCH /api/admin/pages* 响应）
#[derive(Debug, Clone, Serialize)]
pub struct PageAdmin {
    pub id: i64,
    pub title: String,
    pub slug: String,
    pub kind: String,
    pub content_md: String,
    pub enabled: bool,
    pub sort_order: i64,
    pub built_in: bool,
    pub links: Vec<PageLink>,
    pub created_at: String,
    pub updated_at: String,
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

/// POST /api/admin/uploads 响应（契约 UploadResult）；filename 仅回显原始文件名；
/// id 为对应 media 记录 id（契约「媒体库」条款，2026-10-04 新增）
#[derive(Debug, Clone, Serialize)]
pub struct UploadResult {
    pub id: i64,
    pub url: String,
    pub size: u64,
    pub filename: String,
}

/// GET /api/admin/media 列表条目（契约 MediaItem，2026-10-04 新增）；
/// width/height 为图片头解析结果，解析失败为 null
#[derive(Debug, Clone, Serialize)]
pub struct MediaItem {
    pub id: i64,
    pub url: String,
    pub filename: String,
    pub size: i64,
    pub mime: String,
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub created_at: String,
}

/// GET /api/site/settings 响应（契约 SiteSettingsPublic；不含 base_url 等敏感字段）
#[derive(Debug, Clone, Serialize)]
pub struct SiteSettingsPublic {
    pub title: String,
    pub subtitle: String,
    pub description: String,
    pub icp_number: String,
    pub footer_text: String,
    pub per_page: i64,
}

/// GET/PUT /api/admin/site/settings 响应（契约 SiteSettingsAdmin = Public + base_url）
#[derive(Debug, Clone, Serialize)]
pub struct SiteSettingsAdmin {
    #[serde(flatten)]
    pub public: SiteSettingsPublic,
    pub base_url: String,
}

impl From<&crate::settings::SiteSettings> for SiteSettingsPublic {
    fn from(s: &crate::settings::SiteSettings) -> Self {
        Self {
            title: s.title.clone(),
            subtitle: s.subtitle.clone(),
            description: s.description.clone(),
            icp_number: s.icp_number.clone(),
            footer_text: s.footer_text.clone(),
            per_page: s.per_page,
        }
    }
}

impl From<&crate::settings::SiteSettings> for SiteSettingsAdmin {
    fn from(s: &crate::settings::SiteSettings) -> Self {
        Self {
            public: s.into(),
            base_url: s.base_url.clone(),
        }
    }
}

/// 最近一次邮件发送尝试的结果（契约「邮件通知」；内存态、不落库，message 不含密码）
#[derive(Debug, Clone, Serialize)]
pub struct LastSendResult {
    pub ok: bool,
    pub message: String,
    /// RFC3339 UTC（全库时间戳惯例）
    pub at: String,
}

/// GET/PUT /api/admin/smtp 响应（契约 SmtpSettingsAdmin；**永不返回密码**，
/// 只有布尔 has_password）
#[derive(Debug, Clone, Serialize)]
pub struct SmtpSettingsAdmin {
    pub enabled: bool,
    pub host: String,
    pub port: i64,
    pub username: String,
    pub from_name: String,
    pub from_email: String,
    pub to_email: String,
    pub tls: String,
    pub has_password: bool,
    pub last_result: Option<LastSendResult>,
}

/// PUT /api/admin/smtp 请求体（部分更新：缺失/null 字段保持原值）
#[derive(Debug, Clone, Default, Deserialize)]
pub struct SmtpSettingsBody {
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub host: Option<String>,
    #[serde(default)]
    pub port: Option<i64>,
    #[serde(default)]
    pub username: Option<String>,
    #[serde(default)]
    pub from_name: Option<String>,
    #[serde(default)]
    pub from_email: Option<String>,
    #[serde(default)]
    pub to_email: Option<String>,
    #[serde(default)]
    pub tls: Option<String>,
}

impl SmtpSettingsBody {
    /// 合并到现有设置（字符串统一 trim；tls 小写化）；校验由 mailer::validate 负责
    pub fn apply_to(self, base: &crate::mailer::SmtpSettings) -> crate::mailer::SmtpSettings {
        crate::mailer::SmtpSettings {
            enabled: self.enabled.unwrap_or(base.enabled),
            host: self.host.map_or_else(|| base.host.clone(), |v| v.trim().to_string()),
            port: self.port.unwrap_or(base.port),
            username: self
                .username
                .map_or_else(|| base.username.clone(), |v| v.trim().to_string()),
            from_name: self
                .from_name
                .map_or_else(|| base.from_name.clone(), |v| v.trim().to_string()),
            from_email: self
                .from_email
                .map_or_else(|| base.from_email.clone(), |v| v.trim().to_string()),
            to_email: self
                .to_email
                .map_or_else(|| base.to_email.clone(), |v| v.trim().to_string()),
            tls: self
                .tls
                .map_or_else(|| base.tls.clone(), |v| v.trim().to_ascii_lowercase()),
        }
    }
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
    /// 回复的父评论 id（可选；契约「评论回复」条款）。
    /// 校验：父存在、同目标（target_type+目标 id）、status=approved；
    /// 父本身有 parent_id 时两级归一化：parent_id 改写为顶级祖先、被回复人进 reply_to_id
    #[serde(default)]
    pub parent_id: Option<i64>,
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
    /// "draft" | "published" | "scheduled"（POST 必填；PUT 缺省保持原值）
    #[serde(default)]
    pub status: Option<String>,
    /// 置顶（契约「文章置顶与定时发布」条款）：POST 缺省 false；PUT 缺省保持原值
    #[serde(default)]
    pub is_sticky: Option<bool>,
    /// 计划发布时间（RFC3339）：仅 status=scheduled 时接受——POST 必填且须为未来时间；
    /// PUT 提供时为「改期」（同样校验未来时间）。其余状态忽略此字段
    #[serde(default)]
    pub published_at: Option<String>,
}

/// PATCH /api/admin/posts/:id/sticky 请求体（行内快捷置顶/取消置顶）
#[derive(Debug, Clone, Deserialize)]
pub struct StickyBody {
    pub is_sticky: bool,
}

fn de_opt_opt<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::Deserialize<'de>,
{
    Ok(Some(Option::deserialize(deserializer)?))
}

/// PUT /api/admin/site/settings 请求体（全量更新语义；可选字段缺失/null 视为空串）
#[derive(Debug, Clone, Deserialize)]
pub struct SiteSettingsBody {
    pub title: String,
    #[serde(default)]
    pub subtitle: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub icp_number: Option<String>,
    #[serde(default)]
    pub footer_text: Option<String>,
    pub per_page: i64,
    #[serde(default)]
    pub base_url: Option<String>,
}

impl SiteSettingsBody {
    /// 转成领域对象（统一 trim；base_url 去尾 /），校验由 settings::validate 负责
    pub fn into_settings(self) -> crate::settings::SiteSettings {
        let trim = |v: Option<String>| v.unwrap_or_default().trim().to_string();
        crate::settings::SiteSettings {
            title: self.title.trim().to_string(),
            subtitle: trim(self.subtitle),
            description: trim(self.description),
            icp_number: trim(self.icp_number),
            footer_text: self.footer_text.unwrap_or_default().trim().to_string(),
            per_page: self.per_page,
            base_url: trim(self.base_url).trim_end_matches('/').to_string(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct NameBody {
    pub name: String,
}

/// POST/PUT /api/admin/pages 请求体（POST 必填 title/content_md；PUT 全部可选）。
/// kind 不可改：请求体不接受该字段（契约「页面」条款）
#[derive(Debug, Clone, Deserialize)]
pub struct PageBody {
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub slug: Option<String>,
    #[serde(default)]
    pub content_md: Option<String>,
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub sort_order: Option<i64>,
    /// 全量替换语义（按数组顺序重写 sort_order）；仅 kind=links 页面接受，其余 kind 忽略
    #[serde(default)]
    pub links: Option<Vec<PageLinkBody>>,
}

/// 友情链接单条入参（sort_order 由服务端按数组顺序重写，客户端传值忽略）
#[derive(Debug, Clone, Deserialize)]
pub struct PageLinkBody {
    pub name: String,
    pub url: String,
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CommentStatusBody {
    pub status: String,
}

// ---------- 点赞（契约「浏览量与点赞」条款） ----------

/// 三接口统一响应：{likes: 新总数, liked: 当前访客是否已赞}
#[derive(Debug, Clone, Serialize)]
pub struct LikeResult {
    pub likes: i64,
    pub liked: bool,
}

/// POST /api/posts/:slug/like 请求体；liker_key 缺失（字段不存在 → serde 拒绝 → 422）
/// 或非法长度（trim 后为空 / >64 字符 → handler 校验 422）
#[derive(Debug, Clone, Deserialize)]
pub struct LikeBody {
    pub liker_key: String,
}

/// GET/DELETE /api/posts/:slug/like 查询参数（DELETE 亦可走 JSON body，见 LikeBody）
#[derive(Debug, Clone, Deserialize)]
pub struct LikeQuery {
    #[serde(default)]
    pub liker_key: Option<String>,
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
    let per_page = per_page.unwrap_or_else(default_per_page).clamp(1, 100);
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
    /// recent（默认）按 is_sticky DESC, published_at DESC；hot 按
    /// view_count DESC, comment_count DESC, published_at DESC
    /// （热门文章组件数据源；契约「浏览量与点赞」条款）；其他值 → 422 validation_error
    #[serde(default)]
    pub order: Option<String>,
}

/// GET /api/admin/media 查询参数（契约「媒体库」条款；分页口径同 normalize_paging）
#[derive(Debug, Clone, Deserialize)]
pub struct MediaQuery {
    #[serde(default)]
    pub page: Option<i64>,
    #[serde(default)]
    pub per_page: Option<i64>,
}

/// GET /api/search 查询参数；q 缺失或 trim 后为空 → 400 validation_error
#[derive(Debug, Clone, Deserialize)]
pub struct SearchQuery {
    #[serde(default)]
    pub q: Option<String>,
    #[serde(default)]
    pub page: Option<i64>,
    #[serde(default)]
    pub per_page: Option<i64>,
}

/// GET /api/posts/:slug/related 查询参数（契约「相关文章推荐」条款）：
/// limit 用字符串接收后手动校验——声明为 Option<i64> 时非数字会被 axum Query
/// 拒绝成 400 纯文本，不符合契约「非数字 → 422 validation_error」的要求
#[derive(Debug, Clone, Deserialize)]
pub struct RelatedQuery {
    #[serde(default)]
    pub limit: Option<String>,
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
