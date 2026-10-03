//! 页面（Page）领域逻辑：内置页定义与幂等注入、友情链接存取与校验、行读取辅助。
//!
//! 契约「页面」条款（2026-10-03 新增）：
//! - 页面是站点级单页实体（关于/留言板/友情链接/自定义），带 slug、enabled、sort_order；
//! - 内置页面（built_in=1）安装时注入，**启动路径也按 slug 幂等补齐**（已存在的行绝不覆盖，
//!   管理员的编辑/停用不受影响）；不可删除，可停用、可改标题/内容/slug/排序/链接；
//! - 友情链接存独立 `page_links` 表（name/url/description/sort_order）；
//! - SQLite/MySQL 共用 SQL（Any 驱动，`?` 占位符），时间戳 RFC3339 UTC 文本。

use sqlx::any::AnyRow;
use sqlx::{AnyConnection, AnyPool, Row};

use crate::error::{ApiError, ApiResult};
use crate::handlers::helpers::render_markdown;
use crate::models::{PageAdmin, PageLink, PageLinkBody, PageSummary};
use crate::state::{last_insert_id, now_rfc3339};

/// 页面 kind 合法取值（创建恒为 custom；kind 创建后不可改）
pub const KIND_CUSTOM: &str = "custom";
pub const KIND_MESSAGE_BOARD: &str = "message_board";
pub const KIND_LINKS: &str = "links";

/// 内置页面定义（安装时注入；slug 即幂等键）
struct BuiltInPage {
    slug: &'static str,
    title: &'static str,
    kind: &'static str,
    sort_order: i64,
    content_md: &'static str,
    /// 友情链接示例数据（仅 kind=links 的内置页非空）
    links: &'static [(&'static str, &'static str, &'static str)],
}

const ABOUT_MD: &str = r##"欢迎来到本站的**关于**页面。

## 关于本站

本站由 reedblog 驱动——一个 Rust（Axum + SQLx）与 React（Vite + Tailwind）构建的轻量博客系统，
一份 SQL 同时支持 SQLite 与 MySQL。

### 关于我

在这里介绍你自己：

- 你是谁，做什么工作
- 为什么开这个博客
- 想记录和分享什么

> 这是一段内置示例内容，你可以在后台「页面管理」中随时编辑它。
"##;

const GUESTBOOK_MD: &str = r##"这里是本站的**留言板**，欢迎写下你想说的话。

- 分享你对本站内容的想法
- 提出建议或指正错误
- 或者只是打个招呼

留言采用**先发后审**模式：提交后立即展示，管理员可在后台隐藏不当内容。
"##;

const LINKS_MD: &str = r##"这里是本站的**友情链接**。

下方的链接由管理员在后台「页面管理」中维护（名称 / URL / 描述 / 排序），
如需交换友链，请通过留言板或邮件联系。
"##;

/// 三个内置页面（sort_order 依次 10/20/30；友情链接附示例链接）
const BUILT_IN_PAGES: &[BuiltInPage] = &[
    BuiltInPage {
        slug: "about",
        title: "关于",
        kind: KIND_CUSTOM,
        sort_order: 10,
        content_md: ABOUT_MD,
        links: &[],
    },
    BuiltInPage {
        slug: "guestbook",
        title: "留言板",
        kind: KIND_MESSAGE_BOARD,
        sort_order: 20,
        content_md: GUESTBOOK_MD,
        links: &[],
    },
    BuiltInPage {
        slug: "links",
        title: "友情链接",
        kind: KIND_LINKS,
        sort_order: 30,
        content_md: LINKS_MD,
        links: &[
            (
                "Rust 官网",
                "https://www.rust-lang.org",
                "一门赋予每个人构建可靠且高效软件的语言",
            ),
            (
                "Axum",
                "https://github.com/tokio-rs/axum",
                "基于 Tokio 生态的人体工学 Web 框架",
            ),
            (
                "React",
                "https://react.dev",
                "用于构建用户界面的 JavaScript 库",
            ),
        ],
    },
];

/// enabled/built_in 列 SQLite 返回 INTEGER、MySQL TINYINT(1) 可能返回 BOOL，两者都兼容
/// （与 plugins 表 enabled 列同款处理）
pub fn row_bool(r: &AnyRow, col: &str) -> bool {
    if let Ok(b) = r.try_get::<bool, _>(col) {
        return b;
    }
    r.try_get::<i64, _>(col).unwrap_or(0) != 0
}

/// 幂等注入内置页面：按 slug 判断，缺失才插入（绝不覆盖/改动已存在的行，
/// 管理员对内置页的编辑与停用状态因此得以保留）。
/// 安装路径与正常启动路径（connect_pool）都调用；失败由调用方记 warning，不阻断。
pub async fn ensure_builtin_pages(
    db_type: &str,
    conn: &mut AnyConnection,
) -> Result<(), sqlx::Error> {
    for bp in BUILT_IN_PAGES {
        let exists: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM pages WHERE slug = ?")
            .bind(bp.slug)
            .fetch_one(&mut *conn)
            .await?;
        if exists > 0 {
            continue;
        }
        let now = now_rfc3339();
        let content_html = render_markdown(bp.content_md);
        sqlx::query(
            "INSERT INTO pages (title, slug, kind, content_md, content_html, enabled, \
             sort_order, built_in, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, 1, ?, 1, ?, ?)",
        )
        .bind(bp.title)
        .bind(bp.slug)
        .bind(bp.kind)
        .bind(bp.content_md)
        .bind(&content_html)
        .bind(bp.sort_order)
        .bind(&now)
        .bind(&now)
        .execute(&mut *conn)
        .await?;
        let page_id = last_insert_id(conn, db_type).await?;
        for (i, (name, url, desc)) in bp.links.iter().enumerate() {
            sqlx::query(
                "INSERT INTO page_links (page_id, name, url, description, sort_order) \
                 VALUES (?, ?, ?, ?, ?)",
            )
            .bind(page_id)
            .bind(*name)
            .bind(*url)
            .bind(*desc)
            .bind(((i + 1) * 10) as i64)
            .execute(&mut *conn)
            .await?;
        }
    }
    Ok(())
}

/// 查某页面的友情链接（sort_order ASC, id ASC；公开详情与管理端共用）
pub async fn fetch_page_links(pool: &AnyPool, page_id: i64) -> ApiResult<Vec<PageLink>> {
    let rows = sqlx::query(
        "SELECT id, name, url, description, sort_order FROM page_links \
         WHERE page_id = ? ORDER BY sort_order ASC, id ASC",
    )
    .bind(page_id)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .iter()
        .map(|r| PageLink {
            id: r.get::<i64, _>("id"),
            name: r.get::<String, _>("name"),
            url: r.get::<String, _>("url"),
            description: r.get::<String, _>("description"),
            sort_order: r.get::<i64, _>("sort_order"),
        })
        .collect())
}

/// 校验单条链接（契约「页面」条款）：name/url trim 后非空，
/// name ≤100 字符、url ≤500 且必须为 http/https 绝对 URL、description ≤500 字符
pub fn validate_link(name: &str, url: &str, description: &str) -> ApiResult<()> {
    if name.is_empty() {
        return Err(ApiError::validation("链接名称不能为空"));
    }
    if name.chars().count() > 100 {
        return Err(ApiError::validation("链接名称不能超过 100 字符"));
    }
    if url.is_empty() {
        return Err(ApiError::validation("链接 URL 不能为空"));
    }
    if url.chars().count() > 500 {
        return Err(ApiError::validation("链接 URL 不能超过 500 字符"));
    }
    let parsed = url::Url::parse(url).map_err(|_| ApiError::validation("链接 URL 格式无效"))?;
    if !matches!(parsed.scheme(), "http" | "https") || parsed.host().is_none() {
        return Err(ApiError::validation("链接 URL 必须是 http/https 绝对地址"));
    }
    if description.chars().count() > 500 {
        return Err(ApiError::validation("链接描述不能超过 500 字符"));
    }
    Ok(())
}

/// 全量替换某页面的友情链接（按数组顺序重写 sort_order = 10, 20, 30…）
pub async fn replace_page_links(
    pool: &AnyPool,
    db_type: &str,
    page_id: i64,
    links: &[PageLinkBody],
) -> ApiResult<()> {
    // 先统一校验，再落库（避免半途失败留下不完整数据）
    let cleaned: Vec<(String, String, String)> = links
        .iter()
        .map(|l| {
            (
                l.name.trim().to_string(),
                l.url.trim().to_string(),
                l.description.clone().unwrap_or_default().trim().to_string(),
            )
        })
        .collect();
    for (name, url, desc) in &cleaned {
        validate_link(name, url, desc)?;
    }

    let mut conn = pool.acquire().await?;
    sqlx::query("DELETE FROM page_links WHERE page_id = ?")
        .bind(page_id)
        .execute(&mut *conn)
        .await?;
    for (i, (name, url, desc)) in cleaned.iter().enumerate() {
        sqlx::query(
            "INSERT INTO page_links (page_id, name, url, description, sort_order) \
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(page_id)
        .bind(name)
        .bind(url)
        .bind(desc)
        .bind(((i + 1) * 10) as i64)
        .execute(&mut *conn)
        .await?;
    }
    let _ = db_type; // 插入无需回读 id；保留参数以与调用方 db_type 语义一致
    Ok(())
}

/// 管理端页面列表/详情的列集（content_html 不回读：公开详情实时渲染，管理端只需 content_md）
pub const PAGE_COLUMNS: &str =
    "id, title, slug, kind, content_md, enabled, sort_order, built_in, created_at, updated_at";

/// 行 → PageSummary（公开列表）
pub fn row_to_page_summary(r: &AnyRow) -> PageSummary {
    PageSummary {
        id: r.get::<i64, _>("id"),
        title: r.get::<String, _>("title"),
        slug: r.get::<String, _>("slug"),
        kind: r.get::<String, _>("kind"),
        sort_order: r.get::<i64, _>("sort_order"),
    }
}

/// 行 → PageAdmin（links 另查后拼装）
pub fn row_to_page_admin(r: &AnyRow, links: Vec<PageLink>) -> PageAdmin {
    PageAdmin {
        id: r.get::<i64, _>("id"),
        title: r.get::<String, _>("title"),
        slug: r.get::<String, _>("slug"),
        kind: r.get::<String, _>("kind"),
        content_md: r.get::<String, _>("content_md"),
        enabled: row_bool(r, "enabled"),
        sort_order: r.get::<i64, _>("sort_order"),
        built_in: row_bool(r, "built_in"),
        links,
        created_at: r.get::<String, _>("created_at"),
        updated_at: r.get::<String, _>("updated_at"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_pages_are_well_formed() {
        assert_eq!(BUILT_IN_PAGES.len(), 3);
        let kinds: Vec<&str> = BUILT_IN_PAGES.iter().map(|p| p.kind).collect();
        assert_eq!(kinds, vec![KIND_CUSTOM, KIND_MESSAGE_BOARD, KIND_LINKS]);
        // slug 唯一、sort_order 严格递增（导航顺序稳定）
        let mut slugs = std::collections::HashSet::new();
        let mut prev = i64::MIN;
        for p in BUILT_IN_PAGES {
            assert!(slugs.insert(p.slug), "内置页 slug 重复: {}", p.slug);
            assert!(p.sort_order > prev);
            prev = p.sort_order;
            assert!(!p.title.trim().is_empty());
            assert!(!p.content_md.trim().is_empty());
        }
        // 仅 links 页带示例链接，且链接全部合法
        for p in BUILT_IN_PAGES {
            if p.kind == KIND_LINKS {
                assert!(!p.links.is_empty());
            } else {
                assert!(p.links.is_empty());
            }
            for (name, url, desc) in p.links {
                validate_link(name, url, desc).unwrap();
            }
        }
    }

    #[test]
    fn link_validation_rejects_bad_input() {
        assert!(validate_link("", "https://a.com", "").is_err());
        assert!(validate_link("名称", "", "").is_err());
        assert!(validate_link("名称", "not a url", "").is_err());
        assert!(validate_link("名称", "javascript:alert(1)", "").is_err());
        assert!(validate_link("名称", "ftp://a.com/x", "").is_err());
        assert!(validate_link("名称", "https://a.com", &"描".repeat(501)).is_err());
        assert!(validate_link(&"名".repeat(101), "https://a.com", "").is_err());
        // 合法输入
        assert!(validate_link("名称", "https://a.com/x?y=1", "描述").is_ok());
        assert!(validate_link("名称", "http://a.com", "").is_ok());
    }
}
