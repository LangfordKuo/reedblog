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
use crate::pages::row_bool;
use crate::state::{now_rfc3339, require_pool, AppState};

use super::helpers::{
    create_comment_pipeline, derive_excerpt, escape_like, fetch_post_tags, make_snippet,
    md_to_plain_text, render_markdown, row_to_comment_pub, split_search_terms,
    PUBLIC_COMMENT_COLUMNS, PUBLIC_COMMENT_FROM, PUBLIC_COMMENT_THREAD_FILTER, VISIBLE_POST_SQL,
};

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
        is_sticky: row_bool(r, "is_sticky"),
    })
}

// content_md 仅用于 excerpt 为空时推导摘要，不出现在 PostPublic 响应里（契约：列表不含 content_md）
// comment_count 口径与公开评论列表一致（契约「评论回复」条款）：只统计前台可见评论——
// approved、target_type='post'（post_id 列复用为通用目标 id，须防页面留言串号）、
// 且线程可见（hidden 顶级评论的子回复不计；线程内所有 visible 评论都计数）
// is_sticky 为置顶标记（契约「文章置顶与定时发布」条款，2026-10-03 新增）
const PUBLIC_POST_COLUMNS: &str = "p.id, p.title, p.slug, p.excerpt, p.content_md, p.category_id, \
     c.name AS category_name, p.published_at, p.is_sticky, \
     (SELECT COUNT(*) FROM comments m WHERE m.target_type = 'post' AND m.post_id = p.id \
      AND m.status = 'approved' \
      AND (m.parent_id IS NULL OR EXISTS (SELECT 1 FROM comments mp WHERE mp.id = m.parent_id \
          AND mp.status = 'approved'))) AS comment_count";

/// 公开列表分页归一化：显式 per_page 优先（钳 1~100）；未传时默认值取站点设置的
/// per_page（契约「总则-分页」2026-10-03 条款）
async fn normalize_public_paging(
    pool: &AnyPool,
    state: &AppState,
    page: Option<i64>,
    per_page: Option<i64>,
) -> ApiResult<(i64, i64)> {
    if per_page.is_some() {
        return Ok(normalize_paging(page, per_page));
    }
    let settings = crate::settings::load(pool, state).await?;
    let (page, _) = normalize_paging(page, None);
    Ok((page, settings.per_page.clamp(1, 100)))
}

/// GET /api/posts?page&per_page&tag&category&year&month → 分页 [PostPublic]
pub async fn list_posts(
    State(state): State<AppState>,
    Query(q): Query<PostsQuery>,
) -> ApiResult<Json<Page<PostPublic>>> {
    let (pool, _db_type) = require_pool(&state).await?;
    let (page, per_page) = normalize_public_paging(&pool, &state, q.page, q.per_page).await?;

    // 公开可见性（契约「文章置顶与定时发布」条款）：published 恒可见，
    // scheduled 到点可见——:now 必须是 where_sql 的第一个绑定参数
    let mut where_sql = format!("WHERE {VISIBLE_POST_SQL}");
    let mut params: Vec<String> = vec![now_rfc3339()];

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

    // 排序：recent（默认）按 is_sticky DESC, published_at DESC（置顶在前，契约 2026-10-03
    // 置顶条款；tag/category/year/month 过滤后的标签/分类/归档列表同此规则）；
    // hot 按 comment_count DESC, published_at DESC（不受置顶影响；comment_count 为
    // PUBLIC_POST_COLUMNS 中的 SELECT 别名，SQLite/MySQL 均支持按别名排序）
    let order_by = match q.order.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        None | Some("recent") => "p.is_sticky DESC, p.published_at DESC",
        Some("hot") => "comment_count DESC, p.published_at DESC",
        Some(other) => {
            return Err(ApiError::validation(format!(
                "order '{other}' 非法（须为 recent 或 hot）"
            )))
        }
    };

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
         ORDER BY {order_by} LIMIT ? OFFSET ?"
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
    let (page, per_page) = normalize_public_paging(&pool, &state, q.page, q.per_page).await?;

    // 公开可见性（含到点的 scheduled）；:now 为第一个绑定参数。
    // 排序保持 published_at DESC——搜索不受置顶影响（契约「文章置顶与定时发布」条款）
    let mut where_sql = format!("WHERE {VISIBLE_POST_SQL}");
    let mut params: Vec<String> = vec![now_rfc3339()];
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

/// GET /api/posts/:slug → PostDetail；不存在/未公开可见（草稿、未到点的 scheduled）
/// → 404 not_found
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
         WHERE p.slug = ? AND {VISIBLE_POST_SQL}"
    );
    let row = sqlx::query(&sql)
        .bind(&slug)
        .bind(now_rfc3339())
        .fetch_optional(&pool)
        .await?;
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

/// 找公开可见文章的 id（评论接口共用，即「评论目标可见性」）；
/// 不存在/未公开可见（草稿、未到点的 scheduled）→ 404
async fn find_published_post(pool: &AnyPool, slug: &str) -> ApiResult<i64> {
    let sql = format!("SELECT p.id FROM posts p WHERE p.slug = ? AND {VISIBLE_POST_SQL}");
    let row = sqlx::query(&sql)
        .bind(slug)
        .bind(now_rfc3339())
        .fetch_optional(pool)
        .await?;
    Ok(row.ok_or_else(ApiError::not_found)?.get::<i64, _>("id"))
}

/// GET /api/posts/:slug/comments → [CommentPub]（仅 approved 且线程可见，时间 ASC, id ASC）
///
/// 仍为平铺数组（契约「评论回复」条款）：每项带 parent_id/reply_to_id/reply_to_name，
/// 两级树由前端组装；hidden 顶级评论的子回复一并排除（PUBLIC_COMMENT_THREAD_FILTER）。
pub async fn list_comments(
    State(state): State<AppState>,
    Path(slug): Path<String>,
) -> ApiResult<Json<Vec<CommentPub>>> {
    let (pool, _db_type) = require_pool(&state).await?;
    let post_id = find_published_post(&pool, &slug).await?;
    let sql = format!(
        "SELECT {PUBLIC_COMMENT_COLUMNS} {PUBLIC_COMMENT_FROM} \
         WHERE c.target_type = 'post' AND c.post_id = ? AND c.status = 'approved' \
         AND {PUBLIC_COMMENT_THREAD_FILTER} \
         ORDER BY c.created_at ASC, c.id ASC"
    );
    let rows = sqlx::query(&sql).bind(post_id).fetch_all(&pool).await?;
    Ok(Json(rows.iter().map(row_to_comment_pub).collect()))
}

/// POST /api/posts/:slug/comments → 201 CommentPub（先发后审：创建即 approved）
///
/// 走评论共用创建管线（helpers::create_comment_pipeline）：body 可选 parent_id
/// （回复/楼中楼），父评论校验与两级归一化、comment.before_create 钩子链
/// （block → 403 comment_blocked，ctx 带 parent_id/reply_to_id）见契约「评论回复」条款。
pub async fn create_comment(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    body: ValidJson<CreateCommentRequest>,
) -> ApiResult<(StatusCode, Json<CommentPub>)> {
    let Json(req) = body.map_err(ApiError::from)?;
    let (pool, db_type) = require_pool(&state).await?;
    let post_id = find_published_post(&pool, &slug).await?;
    let created = create_comment_pipeline(
        &state,
        &pool,
        &db_type,
        "post",
        post_id,
        &slug,
        req,
        "评论被插件拦截",
    )
    .await?;
    Ok((StatusCode::CREATED, Json(created)))
}

/// GET /api/tags → [Tag]（post_count 只统计公开可见文章：published + 到点的 scheduled）
pub async fn list_tags(State(state): State<AppState>) -> ApiResult<Json<Vec<Tag>>> {
    let (pool, _db_type) = require_pool(&state).await?;
    let sql = format!(
        "SELECT t.id, t.name, COUNT(p.id) AS post_count FROM tags t \
         LEFT JOIN post_tags pt ON pt.tag_id = t.id \
         LEFT JOIN posts p ON p.id = pt.post_id AND {VISIBLE_POST_SQL} \
         GROUP BY t.id, t.name ORDER BY t.name"
    );
    let rows = sqlx::query(&sql)
        .bind(now_rfc3339())
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

/// GET /api/categories → [Category]（post_count 只统计公开可见文章）
pub async fn list_categories(State(state): State<AppState>) -> ApiResult<Json<Vec<Category>>> {
    let (pool, _db_type) = require_pool(&state).await?;
    let sql = format!(
        "SELECT c.id, c.name, COUNT(p.id) AS post_count FROM categories c \
         LEFT JOIN posts p ON p.category_id = c.id AND {VISIBLE_POST_SQL} \
         GROUP BY c.id, c.name ORDER BY c.name"
    );
    let rows = sqlx::query(&sql)
        .bind(now_rfc3339())
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

/// GET /api/archive → [{year, month, count}]，仅公开可见文章，按年月 DESC
/// （scheduled 到点后按计划时间所在年月计入；排序不受置顶影响——归档本身即纯时间维度）
pub async fn archive(State(state): State<AppState>) -> ApiResult<Json<Vec<ArchiveEntry>>> {
    let (pool, _db_type) = require_pool(&state).await?;
    let sql = format!(
        "SELECT SUBSTR(p.published_at, 1, 4) AS y, SUBSTR(p.published_at, 6, 2) AS m, \
         COUNT(*) AS cnt FROM posts p \
         WHERE {VISIBLE_POST_SQL} AND p.published_at IS NOT NULL \
         GROUP BY SUBSTR(p.published_at, 1, 4), SUBSTR(p.published_at, 6, 2) \
         ORDER BY SUBSTR(p.published_at, 1, 4) DESC, SUBSTR(p.published_at, 6, 2) DESC"
    );
    let rows = sqlx::query(&sql)
        .bind(now_rfc3339())
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
