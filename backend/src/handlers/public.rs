//! 站点公开接口：文章列表/详情、评论读写、标签、分类、归档

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use sqlx::any::AnyRow;
use sqlx::AnyPool;
use sqlx::Row;

use crate::error::{ApiError, ApiResult, ValidJson};
use crate::models::{
    normalize_paging, ArchiveEntry, Category, CategoryRef, CommentPub, CreateCommentRequest, Page,
    PostDetail, PostPublic, PostsQuery, SearchQuery, SearchResult, Tag,
};
use crate::state::{now_rfc3339, require_pool, AppState};

use super::helpers::{
    derive_excerpt, escape_like, fetch_post_tags, last_insert_id_on, make_snippet,
    md_to_plain_text, render_markdown, split_search_terms,
};
use crate::plugins::CommentDecision;

/// 把文章行（列表/详情共用列集）转成 PostPublic
async fn row_to_post_public(pool: &AnyPool, r: &AnyRow) -> ApiResult<PostPublic> {
    let id = r.get::<i64, _>("id");
    let category_id = r.try_get::<Option<i64>, _>("category_id").unwrap_or(None);
    let category_name = r
        .try_get::<Option<String>, _>("category_name")
        .unwrap_or(None);
    let category = match (category_id, category_name) {
        (Some(cid), Some(name)) => Some(CategoryRef { id: cid, name }),
        _ => None,
    };
    // excerpt 为空时的回退（契约 2026-10-03 条款）：由 content_md 生成纯文本摘要，
    // 不得返回含 Markdown 符号的原文
    let excerpt = r.get::<String, _>("excerpt");
    let excerpt = if excerpt.trim().is_empty() {
        derive_excerpt(&r.try_get::<String, _>("content_md").unwrap_or_default())
    } else {
        excerpt
    };
    Ok(PostPublic {
        id,
        title: r.get::<String, _>("title"),
        slug: r.get::<String, _>("slug"),
        excerpt,
        category,
        tags: fetch_post_tags(pool, id).await?,
        published_at: r
            .try_get::<Option<String>, _>("published_at")
            .unwrap_or(None)
            .unwrap_or_default(),
        comment_count: r.get::<i64, _>("comment_count"),
    })
}

// content_md 仅用于 excerpt 为空时推导摘要，不出现在 PostPublic 响应里（契约：列表不含 content_md）
const PUBLIC_POST_COLUMNS: &str = "p.id, p.title, p.slug, p.excerpt, p.content_md, p.category_id, \
     c.name AS category_name, p.published_at, \
     (SELECT COUNT(*) FROM comments m WHERE m.post_id = p.id AND m.status = 'approved') AS comment_count";

/// GET /api/posts?page&per_page&tag&category&year&month → 分页 [PostPublic]
pub async fn list_posts(
    State(state): State<AppState>,
    Query(q): Query<PostsQuery>,
) -> ApiResult<Json<Page<PostPublic>>> {
    let (pool, _db_type) = require_pool(&state).await?;
    let (page, per_page) = normalize_paging(q.page, q.per_page);

    let mut where_sql = String::from("WHERE p.status = 'published'");
    let mut params: Vec<String> = Vec::new();

    if let Some(tag) = q.tag.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        where_sql.push_str(
            " AND p.id IN (SELECT pt.post_id FROM post_tags pt \
             JOIN tags t ON t.id = pt.tag_id WHERE t.name = ?)",
        );
        params.push(tag.to_string());
    }
    if let Some(cat) = q
        .category
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        where_sql.push_str(" AND c.name = ?");
        params.push(cat.to_string());
    }
    if let Some(year) = q.year {
        where_sql.push_str(" AND SUBSTR(p.published_at, 1, 4) = ?");
        params.push(format!("{year:04}"));
    }
    if let Some(month) = q.month {
        where_sql.push_str(" AND SUBSTR(p.published_at, 6, 2) = ?");
        params.push(format!("{month:02}"));
    }

    // total
    let count_sql = format!(
        "SELECT COUNT(*) FROM posts p LEFT JOIN categories c ON c.id = p.category_id {where_sql}"
    );
    let mut cq = sqlx::query(&count_sql);
    for prm in &params {
        cq = cq.bind(prm.clone());
    }
    let total: i64 = cq.fetch_one(&pool).await?.get(0);

    // 当页数据
    let list_sql = format!(
        "SELECT {PUBLIC_POST_COLUMNS} FROM posts p \
         LEFT JOIN categories c ON c.id = p.category_id {where_sql} \
         ORDER BY p.published_at DESC LIMIT ? OFFSET ?"
    );
    let mut lq = sqlx::query(&list_sql);
    for prm in &params {
        lq = lq.bind(prm.clone());
    }
    let rows = lq
        .bind(per_page)
        .bind((page - 1) * per_page)
        .fetch_all(&pool)
        .await?;

    let mut items = Vec::with_capacity(rows.len());
    for r in &rows {
        items.push(row_to_post_public(&pool, r).await?);
    }

    Ok(Json(Page {
        items,
        total,
        page,
        per_page,
    }))
}

/// GET /api/search?q&page&per_page → 分页 [SearchResult]（契约「全文搜索」条款）
///
/// LIKE 实现（SQLite/MySQL 共用一份 SQL，零迁移，不用 FTS5/MATCH…AGAINST 单方言语法）：
/// q 按空白切分为词条（上限 8 个，多余忽略），每个词条都须命中 title/excerpt/content_md
/// 之一（AND 语义）；词条内 `%`/`_`/`\` 转义后配合 ESCAPE '\' 子句。
/// 仅 published，按 published_at DESC，分页与 /api/posts 相同（normalize_paging）。
pub async fn search_posts(
    State(state): State<AppState>,
    Query(q): Query<SearchQuery>,
) -> ApiResult<Json<Page<SearchResult>>> {
    let (pool, _db_type) = require_pool(&state).await?;
    let terms = split_search_terms(q.q.as_deref().unwrap_or_default());
    if terms.is_empty() {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "validation_error",
            "q 不能为空",
        ));
    }
    let (page, per_page) = normalize_paging(q.page, q.per_page);

    let mut where_sql = String::from("WHERE p.status = 'published'");
    let mut params: Vec<String> = Vec::new();
    for term in &terms {
        where_sql.push_str(
            " AND (p.title LIKE ? ESCAPE '\\' OR p.excerpt LIKE ? ESCAPE '\\' \
             OR p.content_md LIKE ? ESCAPE '\\')",
        );
        let pat = format!("%{}%", escape_like(term));
        params.push(pat.clone());
        params.push(pat.clone());
        params.push(pat);
    }

    // total
    let count_sql = format!("SELECT COUNT(*) FROM posts p {where_sql}");
    let mut cq = sqlx::query(&count_sql);
    for prm in &params {
        cq = cq.bind(prm.clone());
    }
    let total: i64 = cq.fetch_one(&pool).await?.get(0);

    // 当页数据（PUBLIC_POST_COLUMNS 已含 content_md，snippet 推导直接复用）
    let list_sql = format!(
        "SELECT {PUBLIC_POST_COLUMNS} FROM posts p \
         LEFT JOIN categories c ON c.id = p.category_id {where_sql} \
         ORDER BY p.published_at DESC LIMIT ? OFFSET ?"
    );
    let mut lq = sqlx::query(&list_sql);
    for prm in &params {
        lq = lq.bind(prm.clone());
    }
    let rows = lq
        .bind(per_page)
        .bind((page - 1) * per_page)
        .fetch_all(&pool)
        .await?;

    let mut items = Vec::with_capacity(rows.len());
    for r in &rows {
        let content_md = r.try_get::<String, _>("content_md").unwrap_or_default();
        let post = row_to_post_public(&pool, r).await?;
        // snippet：content_md 剥成纯文本（derive_excerpt 同款逻辑）后截窗口；
        // 纯文本无命中（仅 title/excerpt 命中）→ 回退响应中的 excerpt
        let snippet = make_snippet(&md_to_plain_text(&content_md), &terms, &post.excerpt);
        items.push(SearchResult { post, snippet });
    }

    Ok(Json(Page {
        items,
        total,
        page,
        per_page,
    }))
}

/// GET /api/posts/:slug → PostDetail；不存在/未发布 → 404 not_found
///
/// 渲染管线（扩展契约「后端钩子」）：
/// post.before_render 链改写 content_md → Markdown 渲染 → post.after_render 链改写 content_html。
/// 插件运行时错误自动跳过，不阻断响应。
pub async fn get_post(
    State(state): State<AppState>,
    Path(slug): Path<String>,
) -> ApiResult<Json<PostDetail>> {
    let (pool, _db_type) = require_pool(&state).await?;
    let sql = format!(
        "SELECT {PUBLIC_POST_COLUMNS} FROM posts p \
         LEFT JOIN categories c ON c.id = p.category_id \
         WHERE p.slug = ? AND p.status = 'published'"
    );
    let row = sqlx::query(&sql).bind(&slug).fetch_optional(&pool).await?;
    let row = row.ok_or_else(ApiError::not_found)?;
    let content_md = row.get::<String, _>("content_md");
    let post = row_to_post_public(&pool, &row).await?;

    let (render_title, render_md) = state
        .plugins()
        .run_post_before_render(&post.title, &content_md, &post.slug)
        .await;
    let raw_html = render_markdown(&render_md);
    let content_html = state
        .plugins()
        .run_post_after_render(&render_title, &raw_html, &post.slug)
        .await;

    Ok(Json(PostDetail {
        post,
        content_md: render_md,
        content_html,
    }))
}

/// 找已发布文章的 id（评论接口共用）；不存在/未发布 → 404
async fn find_published_post(pool: &AnyPool, slug: &str) -> ApiResult<i64> {
    let row = sqlx::query("SELECT id FROM posts WHERE slug = ? AND status = 'published'")
        .bind(slug)
        .fetch_optional(pool)
        .await?;
    Ok(row.ok_or_else(ApiError::not_found)?.get::<i64, _>("id"))
}

/// GET /api/posts/:slug/comments → [CommentPub]（仅 approved，时间 ASC）
pub async fn list_comments(
    State(state): State<AppState>,
    Path(slug): Path<String>,
) -> ApiResult<Json<Vec<CommentPub>>> {
    let (pool, _db_type) = require_pool(&state).await?;
    let post_id = find_published_post(&pool, &slug).await?;
    let rows = sqlx::query(
        "SELECT id, author_name, content, created_at FROM comments \
         WHERE post_id = ? AND status = 'approved' ORDER BY created_at ASC",
    )
    .bind(post_id)
    .fetch_all(&pool)
    .await?;
    Ok(Json(
        rows.iter()
            .map(|r| CommentPub {
                id: r.get::<i64, _>("id"),
                author_name: r.get::<String, _>("author_name"),
                content: r.get::<String, _>("content"),
                created_at: r.get::<String, _>("created_at"),
            })
            .collect(),
    ))
}

/// POST /api/posts/:slug/comments → 201 CommentPub（先发后审：创建即 approved）
///
/// comment.before_create 钩子链（扩展契约）：任一插件返回 block 立即短路 → 403 comment_blocked
/// （reason 进 message）；allow 可携带修改后的字段。插件运行时错误跳过、不阻断。
pub async fn create_comment(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    body: ValidJson<CreateCommentRequest>,
) -> ApiResult<(StatusCode, Json<CommentPub>)> {
    let Json(req) = body.map_err(ApiError::from)?;
    let (pool, db_type) = require_pool(&state).await?;
    let post_id = find_published_post(&pool, &slug).await?;

    let author_name = req.author_name.trim();
    let content = req.content.trim();
    if author_name.is_empty() {
        return Err(ApiError::validation("author_name 不能为空"));
    }
    if content.is_empty() {
        return Err(ApiError::validation("content 不能为空"));
    }
    let email = req
        .email
        .map(|e| e.trim().to_string())
        .filter(|e| !e.is_empty());

    let (author_name, email, content) = match state
        .plugins()
        .run_comment_before_create(&slug, author_name, email.as_deref(), content)
        .await
    {
        CommentDecision::Block { reason } => {
            let message = if reason.trim().is_empty() {
                "评论被插件拦截".to_string()
            } else {
                reason
            };
            return Err(ApiError::new(
                StatusCode::FORBIDDEN,
                "comment_blocked",
                message,
            ));
        }
        CommentDecision::Allow {
            author_name,
            email,
            content,
        } => (author_name, email, content),
    };
    // 插件改写后的字段仍需满足基本约束
    if author_name.trim().is_empty() {
        return Err(ApiError::validation("author_name 不能为空"));
    }
    if content.trim().is_empty() {
        return Err(ApiError::validation("content 不能为空"));
    }

    let created_at = now_rfc3339();
    let mut conn = pool.acquire().await?;
    sqlx::query(
        "INSERT INTO comments (post_id, author_name, email, content, status, created_at) \
         VALUES (?, ?, ?, ?, 'approved', ?)",
    )
    .bind(post_id)
    .bind(author_name.trim())
    .bind(email.as_deref().map(str::trim).filter(|e| !e.is_empty()))
    .bind(content.trim())
    .bind(&created_at)
    .execute(&mut *conn)
    .await?;
    let id = last_insert_id_on(&mut conn, &db_type).await?;

    Ok((
        StatusCode::CREATED,
        Json(CommentPub {
            id,
            author_name: author_name.trim().to_string(),
            content: content.trim().to_string(),
            created_at,
        }),
    ))
}

/// GET /api/tags → [Tag]（post_count 只统计 published）
pub async fn list_tags(State(state): State<AppState>) -> ApiResult<Json<Vec<Tag>>> {
    let (pool, _db_type) = require_pool(&state).await?;
    let rows = sqlx::query(
        "SELECT t.id, t.name, COUNT(p.id) AS post_count FROM tags t \
         LEFT JOIN post_tags pt ON pt.tag_id = t.id \
         LEFT JOIN posts p ON p.id = pt.post_id AND p.status = 'published' \
         GROUP BY t.id, t.name ORDER BY t.name",
    )
    .fetch_all(&pool)
    .await?;
    Ok(Json(
        rows.iter()
            .map(|r| Tag {
                id: r.get::<i64, _>("id"),
                name: r.get::<String, _>("name"),
                post_count: r.get::<i64, _>("post_count"),
            })
            .collect(),
    ))
}

/// GET /api/categories → [Category]（post_count 只统计 published）
pub async fn list_categories(State(state): State<AppState>) -> ApiResult<Json<Vec<Category>>> {
    let (pool, _db_type) = require_pool(&state).await?;
    let rows = sqlx::query(
        "SELECT c.id, c.name, COUNT(p.id) AS post_count FROM categories c \
         LEFT JOIN posts p ON p.category_id = c.id AND p.status = 'published' \
         GROUP BY c.id, c.name ORDER BY c.name",
    )
    .fetch_all(&pool)
    .await?;
    Ok(Json(
        rows.iter()
            .map(|r| Category {
                id: r.get::<i64, _>("id"),
                name: r.get::<String, _>("name"),
                post_count: r.get::<i64, _>("post_count"),
            })
            .collect(),
    ))
}

/// GET /api/archive → [{year, month, count}]，仅 published，按年月 DESC
pub async fn archive(State(state): State<AppState>) -> ApiResult<Json<Vec<ArchiveEntry>>> {
    let (pool, _db_type) = require_pool(&state).await?;
    let rows = sqlx::query(
        "SELECT SUBSTR(published_at, 1, 4) AS y, SUBSTR(published_at, 6, 2) AS m, \
         COUNT(*) AS cnt FROM posts \
         WHERE status = 'published' AND published_at IS NOT NULL \
         GROUP BY SUBSTR(published_at, 1, 4), SUBSTR(published_at, 6, 2) \
         ORDER BY SUBSTR(published_at, 1, 4) DESC, SUBSTR(published_at, 6, 2) DESC",
    )
    .fetch_all(&pool)
    .await?;
    let mut items = Vec::new();
    for r in &rows {
        let y: i64 = r.get::<String, _>("y").parse().unwrap_or(0);
        let m: i64 = r.get::<String, _>("m").parse().unwrap_or(0);
        items.push(ArchiveEntry {
            year: y,
            month: m,
            count: r.get::<i64, _>("cnt"),
        });
    }
    Ok(Json(items))
}
