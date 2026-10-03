//! 示例数据注入：安装成功后执行一次，让新装好的站点开箱即有内容可看、后台可管理。
//!
//! 硬性约束（契约「安装向导」条款）：
//! - **只在安装流程**（handlers/install.rs 的成功路径）调用一次；
//!   正常启动路径（startup_state / connect_pool）绝不调用，不做「表空就注入」判断；
//! - 注入失败不得导致安装失败：调用方捕获 Err 记 warning 日志后照常完成安装；
//! - SQLite/MySQL 共用一份 SQL（sqlx Any 驱动，`?` 占位符），无任何单方言语法；
//! - 时间戳按项目惯例存 RFC3339 UTC 文本（秒精度，如 `2026-09-20T08:45:00Z`）；
//! - slug 生成与管理端发文流程一致（helpers::slugify；纯中文标题回退 `post-<id>`，插入后回填）；
//! - excerpt 留空的示例文章在注入时用 helpers::derive_excerpt 推导（与管理端发文流程一致）。

use sqlx::AnyConnection;

use crate::handlers::helpers::{derive_excerpt, slugify, temp_slug};
use crate::state::last_insert_id;

/// 示例访客评论（先发后审：注入即 approved，公开可见）
struct SampleComment {
    author_name: &'static str,
    email: Option<&'static str>,
    content: &'static str,
    created_at: &'static str,
}

/// 单篇示例文章（excerpt None = 留空，注入时由正文推导自动摘要）
struct SamplePost {
    title: &'static str,
    content_md: &'static str,
    excerpt: Option<&'static str>,
    category: &'static str,
    tags: &'static [&'static str],
    /// 三篇错开成不同月份，方便验证归档页按月分组
    published_at: &'static str,
    comment: Option<SampleComment>,
}

const SAMPLE_CATEGORIES: &[&str] = &["技术分享", "生活随笔", "默认分类"];

const SAMPLE_TAGS: &[&str] = &["Rust", "前端", "教程", "随笔", "生活"];

const WELCOME_MD: &str = r##"恭喜，你的 reedblog 站点已经安装成功！

## 从这里开始

这是一篇自动注入的**示例文章**，用来展示常用 Markdown 语法的渲染效果，你可以在后台*直接编辑或删除*它。

### 接下来可以做什么

- 在后台「文章」里写下第一篇文章
- 用「分类 / 标签」整理内容结构
- 在「评论」里查看并审核读者留言

也可以按下面的顺序快速上手：

1. 登录后台管理
2. 新建文章并选好分类
3. 点击发布，前台立即可见

### 引用与链接

> 写作是思考的延伸，博客是思考的存档。

更多用法请参考 [reedblog 使用文档](https://example.com/reedblog-docs)。

---

祝写作愉快！
"##;

const AXUM_MD: &str = r##"reedblog 的后端选择了 **Axum + SQLx** 的组合，这篇文章记录几个关键设计。

## 为什么是 Axum

Axum 构建在 Tokio 生态之上，路由与中间件都是普通的 async 函数，组合起来非常直接：

1. 用 `Router::new()` 声明路由
2. 用 `layer` 挂上中间件（CORS、未安装门禁）
3. 用 `with_state` 注入共享状态

### 一段最小示例

```rust
use axum::{routing::get, Router};

async fn health() -> &'static str {
    "ok"
}

#[tokio::main]
async fn main() {
    let app = Router::new().route("/api/health", get(health));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:3000").await.unwrap();
    axum::serve(listener, app).await.unwrap();
}
```

> 提示：`#[tokio::main]` 只是一个宏，展开后就是标准的 Tokio runtime 启动代码。

## 一份 SQL 同时跑 SQLite 和 MySQL

借助 SQLx 的 Any 驱动，同一份 `?` 占位符 SQL 可以同时对接两种数据库；时间戳统一存 RFC3339 UTC 文本，字典序即时间序，避开方言差异。前端则使用 React 与 Vite，通过 JSON 契约与后端通信，详见 [API 契约文档](https://example.com/api-contract)。

---

*下一篇聊聊部署与运维。*
"##;

const ESSAY_MD: &str = r##"周六的早晨没有闹钟，阳光先一步把人叫醒。

## 一杯手冲的时间

水烧到九十度，缓慢画圈注入，咖啡粉一点点膨胀起来，整个厨房都是香气。平时习惯了倍速刷手机，才发现**认真等一杯咖啡**也是一种休息。

### 下午的旧书

从书架深处翻出一本没读完的小说，扉页上还留着几年前的日期。读几页，发一会儿呆：

- 有些书适合一口气读完
- 有些书适合隔几年再读
- 还有些书，只是为了记住买书那天的自己

> 慢下来不是浪费时间，而是把时间还给感受。

窗外的光线从*明亮*转到昏黄，合上书的时候，居然没有一丝愧疚。

---

下周也试着留一个这样的周末。
"##;

/// 三篇示例文章（插入顺序即 id 顺序；发布时间错开在三个不同月份）
const SAMPLE_POSTS: &[SamplePost] = &[
    SamplePost {
        title: "欢迎使用 reedblog",
        content_md: WELCOME_MD,
        excerpt: Some(
            "reedblog 安装成功！这是一篇自动注入的示例文章，演示常用 Markdown 语法的渲染效果，可以在后台直接编辑或删除。",
        ),
        category: "默认分类",
        tags: &["教程", "随笔"],
        published_at: "2026-09-20T08:45:00Z",
        comment: None,
    },
    SamplePost {
        title: "用 Axum 和 SQLx 搭建轻量博客后端",
        content_md: AXUM_MD,
        excerpt: Some(
            "一份最小可行的 Rust Web 后端组合：Axum 负责路由与中间件，SQLx Any 驱动让同一份 SQL 同时支持 SQLite 与 MySQL。",
        ),
        category: "技术分享",
        tags: &["Rust", "前端", "教程"],
        published_at: "2026-08-15T14:20:00Z",
        comment: Some(SampleComment {
            author_name: "林晚风",
            email: Some("linwanfeng@example.com"),
            content: "写得很清楚！我们的小项目也在评估 Axum，Any 驱动同时支持两种数据库这个思路很受启发。请问生产环境下 MySQL 连接池一般设多大？",
            created_at: "2026-08-16T03:12:00Z",
        }),
    },
    SamplePost {
        // excerpt 留空：验证后端自动摘要（剥离 Markdown 的纯文本 ≤200 字符）
        title: "周末随笔：慢下来的时光",
        content_md: ESSAY_MD,
        excerpt: None,
        category: "生活随笔",
        tags: &["随笔", "生活"],
        published_at: "2026-07-08T09:30:00Z",
        comment: None,
    },
];

/// 按唯一 name 查 categories/tags 的 id（table 为模块内静态常量，非用户输入）
async fn lookup_id(conn: &mut AnyConnection, table: &str, name: &str) -> Result<i64, sqlx::Error> {
    let sql = format!("SELECT id FROM {table} WHERE name = ?");
    sqlx::query_scalar::<_, i64>(&sql)
        .bind(name)
        .fetch_one(&mut *conn)
        .await
}

/// 注入全部示例数据：3 分类 + 5 标签 + 3 篇已发布文章（各挂 1 分类 + 2~3 标签）
/// + 1 条公开可见的示例访客评论。
///
/// 前置条件：迁移已跑完（表已建好）、库是安装流程新建的（无重名/重 slug 冲突）。
/// 返回 Err 时调用方只记 warning，不阻断安装（可能留下部分注入的数据，无碍站点运行）。
pub async fn seed_sample_data(db_type: &str, conn: &mut AnyConnection) -> Result<(), sqlx::Error> {
    for name in SAMPLE_CATEGORIES {
        sqlx::query("INSERT INTO categories (name) VALUES (?)")
            .bind(*name)
            .execute(&mut *conn)
            .await?;
    }
    for name in SAMPLE_TAGS {
        sqlx::query("INSERT INTO tags (name) VALUES (?)")
            .bind(*name)
            .execute(&mut *conn)
            .await?;
    }

    for post in SAMPLE_POSTS {
        let category_id = lookup_id(conn, "categories", post.category).await?;

        // excerpt 留空 → 与管理端发文流程同款自动摘要
        let excerpt = match post.excerpt {
            Some(e) => e.to_string(),
            None => derive_excerpt(post.content_md),
        };

        // slug 规则与管理端发文流程一致：ASCII slugify；纯中文标题回退 post-<id>（插入后回填）
        let generated = slugify(post.title);
        let (mut slug, need_backfill) = if generated.is_empty() {
            (temp_slug(), true)
        } else {
            (generated, false)
        };

        sqlx::query(
            "INSERT INTO posts (title, slug, excerpt, content_md, status, category_id, \
             published_at, created_at, updated_at) VALUES (?, ?, ?, ?, 'published', ?, ?, ?, ?)",
        )
        .bind(post.title)
        .bind(&slug)
        .bind(&excerpt)
        .bind(post.content_md)
        .bind(category_id)
        .bind(post.published_at)
        .bind(post.published_at)
        .bind(post.published_at)
        .execute(&mut *conn)
        .await?;
        let id = last_insert_id(conn, db_type).await?;
        if need_backfill {
            slug = format!("post-{id}");
            sqlx::query("UPDATE posts SET slug = ? WHERE id = ?")
                .bind(&slug)
                .bind(id)
                .execute(&mut *conn)
                .await?;
        }

        for tag in post.tags {
            let tag_id = lookup_id(conn, "tags", tag).await?;
            sqlx::query("INSERT INTO post_tags (post_id, tag_id) VALUES (?, ?)")
                .bind(id)
                .bind(tag_id)
                .execute(&mut *conn)
                .await?;
        }

        // 示例访客评论：先发后审模型下创建即 approved，公开可见
        if let Some(cm) = &post.comment {
            sqlx::query(
                "INSERT INTO comments (post_id, author_name, email, content, status, created_at) \
                 VALUES (?, ?, ?, ?, 'approved', ?)",
            )
            .bind(id)
            .bind(cm.author_name)
            .bind(cm.email)
            .bind(cm.content)
            .bind(cm.created_at)
            .execute(&mut *conn)
            .await?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn three_published_posts_in_distinct_months() {
        assert_eq!(SAMPLE_POSTS.len(), 3);
        let months: HashSet<&str> = SAMPLE_POSTS.iter().map(|p| &p.published_at[..7]).collect();
        assert_eq!(
            months.len(),
            3,
            "三篇示例文章应分布在三个不同月份（归档按月分组）"
        );
        for p in SAMPLE_POSTS {
            // 时间戳必须是 RFC3339 UTC 文本（项目惯例）
            let dt = chrono::DateTime::parse_from_rfc3339(p.published_at)
                .unwrap_or_else(|e| panic!("{} 非法 RFC3339: {e}", p.title));
            assert!(dt.to_string().ends_with('Z') || p.published_at.ends_with('Z'));
        }
    }

    #[test]
    fn exactly_one_post_derives_excerpt() {
        let auto: Vec<_> = SAMPLE_POSTS
            .iter()
            .filter(|p| p.excerpt.is_none())
            .collect();
        assert_eq!(auto.len(), 1, "应恰好有 1 篇 excerpt 留空的示例文章");
        let derived = derive_excerpt(auto[0].content_md);
        assert!(!derived.is_empty(), "自动摘要不应为空");
        assert!(
            derived.chars().count() <= 200,
            "自动摘要应 ≤200 字符: {derived}"
        );
        for sym in ['#', '*', '`', '>', '|', '[', ']'] {
            assert!(
                !derived.contains(sym),
                "自动摘要不应含 Markdown 符号 {sym}: {derived}"
            );
        }
        // 其余文章显式 excerpt 非空
        for p in SAMPLE_POSTS.iter().filter(|p| p.excerpt.is_some()) {
            assert!(!p.excerpt.unwrap().trim().is_empty(), "{}", p.title);
        }
    }

    #[test]
    fn taxonomy_references_are_consistent() {
        assert_eq!(SAMPLE_CATEGORIES.len(), 3);
        for p in SAMPLE_POSTS {
            assert!(SAMPLE_CATEGORIES.contains(&p.category), "{}", p.title);
            assert!(
                (2..=3).contains(&p.tags.len()),
                "每篇示例文章应挂 2~3 个标签: {}",
                p.title
            );
            for t in p.tags {
                assert!(SAMPLE_TAGS.contains(t), "未定义的标签 {t}");
            }
            // 标签不重复挂载
            let set: HashSet<&str> = p.tags.iter().copied().collect();
            assert_eq!(set.len(), p.tags.len(), "{}", p.title);
        }
        // 每个示例标签都至少被一篇文章使用（公开标签页无零计数死标签）
        for t in SAMPLE_TAGS {
            assert!(
                SAMPLE_POSTS.iter().any(|p| p.tags.contains(t)),
                "标签 {t} 未被任何示例文章使用"
            );
        }
    }

    #[test]
    fn corpus_covers_common_markdown_syntax() {
        let all: String = SAMPLE_POSTS
            .iter()
            .map(|p| p.content_md)
            .collect::<Vec<_>>()
            .join("\n");
        // 二级/三级标题、无序/有序列表、代码块、引用、链接、加粗斜体、分隔线
        for needle in ["## ", "### ", "- ", "1. ", "```", "> ", "**", "---", "]("] {
            assert!(all.contains(needle), "示例内容应覆盖 {needle:?}");
        }
        // 斜体（单星号成对）与行内代码
        assert!(all.contains("*直接编辑或删除*"), "应含斜体示例");
        assert!(all.contains("`Router::new()`"), "应含行内代码示例");
    }

    #[test]
    fn slugs_follow_existing_generation_rules() {
        // 与管理端发文流程一致：ASCII slugify；纯中文标题回退 post-<id>
        assert_eq!(slugify("欢迎使用 reedblog"), "reedblog");
        assert_eq!(slugify("用 Axum 和 SQLx 搭建轻量博客后端"), "axum-sqlx");
        assert_eq!(slugify("周末随笔：慢下来的时光"), "");
        // 三篇 slug 互不相同（回退篇为 post-<id>，天然唯一）
        let mut slugs: HashSet<String> = HashSet::new();
        for p in SAMPLE_POSTS {
            let gen = slugify(p.title);
            assert!(slugs.insert(gen), "slug 冲突: {}", p.title);
        }
    }

    #[test]
    fn one_realistic_guest_comment_on_published_post() {
        let with_comment: Vec<_> = SAMPLE_POSTS
            .iter()
            .filter(|p| p.comment.is_some())
            .collect();
        assert_eq!(with_comment.len(), 1, "应恰好注入 1 条示例评论");
        let post = with_comment[0];
        let cm = post.comment.as_ref().unwrap();
        assert!(!cm.author_name.trim().is_empty());
        assert!(!cm.content.trim().is_empty());
        assert!(cm.email.unwrap_or_default().contains('@'), "邮箱应逼真");
        // 评论时间戳合法且不早于所在文章的发布时间（RFC3339 字典序即时间序）
        assert!(chrono::DateTime::parse_from_rfc3339(cm.created_at).is_ok());
        assert!(cm.created_at >= post.published_at);
    }
}
