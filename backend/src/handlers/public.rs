//! 站点公开接口：文章列表/详情（含浏览量计数）、点赞、评论读写、标签、分类、归档

use axum::extract::{ConnectInfo, Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use sqlx::any::AnyRow;
use sqlx::AnyPool;
use sqlx::Row;
use std::net::SocketAddr;

use crate::error::{ApiError, ApiResult, ValidJson};
use crate::models::{
    normalize_paging, ArchiveEntry, Category, CategoryRef, CommentPub, CreateCommentRequest,
    LikeBody, LikeQuery, LikeResult, Page, PostDetail, PostNeighbor, PostPublic, PostsQuery,
    SearchQuery, SearchResult, Tag,
};
use crate::pages::row_bool;
use crate::state::{now_rfc3339, require_pool, AppState};
use crate::views::{client_ip, has_bearer, is_bot_ua};

use super::helpers::{
    create_comment_pipeline, derive_excerpt, escape_like, fetch_post_tags, is_unique_violation,
    make_snippet, md_to_plain_text, render_markdown, row_to_comment_pub, split_search_terms,
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
        view_count: r.try_get::<i64, _>("view_count").unwrap_or(0),
        likes: r.try_get::<i64, _>("likes").unwrap_or(0),
    })
}

// content_md 仅用于 excerpt 为空时推导摘要，不出现在 PostPublic 响应里（契约：列表不含 content_md）
// comment_count 口径与公开评论列表一致（契约「评论回复」条款）：只统计前台可见评论——
// approved、target_type='post'（post_id 列复用为通用目标 id，须防页面留言串号）、
// 且线程可见（hidden 顶级评论的子回复不计；线程内所有 visible 评论都计数）
// is_sticky 为置顶标记（契约「文章置顶与定时发布」条款，2026-10-03 新增）
// view_count / likes（契约「浏览量与点赞」条款）：likes 为 post_likes 子查询计数——
// 走 (post_id, liker_key) UNIQUE 索引最左前缀，文章量小，双方言性能可接受
const PUBLIC_POST_COLUMNS: &str = "p.id, p.title, p.slug, p.excerpt, p.content_md, p.category_id, \
     c.name AS category_name, p.published_at, p.is_sticky, p.view_count, \
     (SELECT COUNT(*) FROM post_likes l WHERE l.post_id = p.id) AS likes, \
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
    // hot 按 view_count DESC, comment_count DESC, published_at DESC（契约「浏览量与点赞」
    // 条款；不受置顶影响；comment_count 为 PUBLIC_POST_COLUMNS 中的 SELECT 别名，
    // SQLite/MySQL 均支持按别名排序）
    let order_by = match q.order.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        None | Some("recent") => "p.is_sticky DESC, p.published_at DESC",
        Some("hot") => "p.view_count DESC, comment_count DESC, p.published_at DESC",
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

/// 详情页单个方向的相邻文章（契约「文章上一篇/下一篇」条款，2026-10-04 新增）：
/// `later=false` 取发布时间更早的紧邻一条（prev）、`true` 取更晚的（next）。
/// 排序/比较键为 (published_at, id)（id 做同秒发布的稳定 tiebreak；
/// **不看 is_sticky**——相邻关系是纯时间语义，与列表页的 sticky 优先序无关）。
/// 可见性谓词复用 VISIBLE_POST_SQL（草稿、未到点 scheduled 都不作相邻项），
/// :now 为其绑定参数；`(published_at, id)` 用显式 OR 展开比较，SQLite/MySQL 共用一份 SQL。
/// 只取 title/slug 两列，不查正文。
async fn fetch_post_neighbor(
    pool: &AnyPool,
    published_at: &str,
    id: i64,
    later: bool,
) -> ApiResult<Option<PostNeighbor>> {
    // 更晚 → 取比当前键大的最小者（ASC）；更早 → 取比当前键小的最大者（DESC）
    let (cmp, order) = if later { (">", "ASC") } else { ("<", "DESC") };
    let sql = format!(
        "SELECT p.title, p.slug FROM posts p \
         WHERE {VISIBLE_POST_SQL} \
         AND (p.published_at {cmp} ? OR (p.published_at = ? AND p.id {cmp} ?)) \
         ORDER BY p.published_at {order}, p.id {order} LIMIT 1"
    );
    let row = sqlx::query(&sql)
        .bind(now_rfc3339())
        .bind(published_at)
        .bind(published_at)
        .bind(id)
        .fetch_optional(pool)
        .await?;
    Ok(row.map(|r| PostNeighbor {
        title: r.get::<String, _>("title"),
        slug: r.get::<String, _>("slug"),
    }))
}

/// GET /api/posts/:slug → PostDetail；不存在/未公开可见（草稿、未到点的 scheduled）
/// → 404 not_found
///
/// 浏览量计数（契约「浏览量与点赞」条款）：每次公开命中 view_count + 1（自增 SQL
/// 双方言通用）；带 Bearer 的请求（后台预览）与爬虫 UA 不计数；进程内
/// (ip, post_id) 60 分钟窗口去重——尽力去重、非精确审计，重启清零。
/// 计数成功时响应中的 view_count 含本次（内存 +1，避免二次查询）。
///
/// 渲染管线（扩展契约「后端钩子」）：
/// post.before_render 链改写 content_md → Markdown 渲染 → post.after_render 链改写 content_html。
/// 插件运行时错误自动跳过，不阻断响应。
pub async fn get_post(
    State(state): State<AppState>,
    headers: HeaderMap,
    // 测试路径的裸 axum::serve 不提供 ConnectInfo（rejection → None，走代理头/兜底）；
    // 生产路径 run() 用 into_make_service_with_connect_info 提供直连地址
    connect: Result<ConnectInfo<SocketAddr>, axum::extract::rejection::ExtensionRejection>,
    Path(slug): Path<String>,
) -> ApiResult<Json<PostDetail>> {
    let connect = connect.ok();
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
    let mut post = row_to_post_public(&pool, &row).await?;

    // 计数判定：后台（Bearer）与爬虫不计数；其余按 (ip, post_id) 窗口去重
    let ua = headers
        .get(axum::http::header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if !has_bearer(&headers) && !is_bot_ua(ua) {
        let ip = client_ip(&headers, connect.as_ref());
        if state.view_dedup().should_count(&ip, post.id) {
            sqlx::query("UPDATE posts SET view_count = view_count + 1 WHERE id = ?")
                .bind(post.id)
                .execute(&pool)
                .await?;
            post.view_count += 1;
        }
    }

    let (render_title, render_md) = state
        .plugins()
        .run_post_before_render(&post.title, &content_md, &post.slug)
        .await;
    let raw_html = render_markdown(&render_md);
    let content_html = state
        .plugins()
        .run_post_after_render(&render_title, &raw_html, &post.slug)
        .await;

    // 上一篇/下一篇（契约「文章上一篇/下一篇」条款）：在详情 handler 内查相邻，不新增端点。
    // published_at 为空（异常数据，正常发布流程不会出现）时时间序失据 → 两侧均为 null
    let (prev_post, next_post) = if post.published_at.is_empty() {
        (None, None)
    } else {
        (
            fetch_post_neighbor(&pool, &post.published_at, post.id, false).await?,
            fetch_post_neighbor(&pool, &post.published_at, post.id, true).await?,
        )
    };

    Ok(Json(PostDetail {
        post,
        content_md: render_md,
        content_html,
        prev_post,
        next_post,
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

// ---------- 点赞（契约「浏览量与点赞」条款） ----------

/// liker_key 长度上限（字符）：前端匿名 id 为 UUID（36 字符），留余量；
/// 与 MySQL 列宽 VARCHAR(64) 一致
const LIKER_KEY_MAX_CHARS: usize = 64;

/// liker_key 校验：缺失 / trim 后为空 / 超长 → 422 validation_error
fn validate_liker_key(raw: Option<String>) -> ApiResult<String> {
    let key = raw.map(|s| s.trim().to_string()).unwrap_or_default();
    if key.is_empty() {
        return Err(ApiError::validation(
            "liker_key 必填（前端 localStorage 匿名 id）",
        ));
    }
    if key.chars().count() > LIKER_KEY_MAX_CHARS {
        return Err(ApiError::validation(format!(
            "liker_key 超长（上限 {LIKER_KEY_MAX_CHARS} 字符）"
        )));
    }
    Ok(key)
}

/// 点赞总数 + 当前访客是否已赞（三接口共用响应组装）
async fn like_status(pool: &AnyPool, post_id: i64, liker_key: &str) -> ApiResult<LikeResult> {
    let likes: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM post_likes WHERE post_id = ?")
        .bind(post_id)
        .fetch_one(pool)
        .await?;
    let mine: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM post_likes WHERE post_id = ? AND liker_key = ?")
            .bind(post_id)
            .bind(liker_key)
            .fetch_one(pool)
            .await?;
    Ok(LikeResult {
        likes,
        liked: mine > 0,
    })
}

/// GET /api/posts/:slug/like?liker_key= → {likes, liked}（当前访客是否已赞，
/// 前端进详情页时调用决定按钮初始状态）；文章不可见 → 404；liker_key 非法 → 422
pub async fn get_like(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    Query(q): Query<LikeQuery>,
) -> ApiResult<Json<LikeResult>> {
    let key = validate_liker_key(q.liker_key)?;
    let (pool, _db_type) = require_pool(&state).await?;
    let post_id = find_published_post(&pool, &slug).await?;
    Ok(Json(like_status(&pool, post_id, &key).await?))
}

/// POST /api/posts/:slug/like body {liker_key} → 200 {likes, liked: true}。
/// 重复点赞同 key → 幂等返回当前状态（不报错）：靠 (post_id, liker_key) UNIQUE
/// 约束，冲突（is_unique_violation 双方言判定）即视为已赞
pub async fn like_post(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    body: ValidJson<LikeBody>,
) -> ApiResult<Json<LikeResult>> {
    let Json(req) = body.map_err(ApiError::from)?;
    let key = validate_liker_key(Some(req.liker_key))?;
    let (pool, _db_type) = require_pool(&state).await?;
    let post_id = find_published_post(&pool, &slug).await?;

    let insert =
        sqlx::query("INSERT INTO post_likes (post_id, liker_key, created_at) VALUES (?, ?, ?)")
            .bind(post_id)
            .bind(&key)
            .bind(now_rfc3339())
            .execute(&pool)
            .await;
    if let Err(e) = insert {
        // UNIQUE 冲突 = 该访客已赞过（并发/重复请求），幂等成功；其余错误上抛
        if !is_unique_violation(&e) {
            return Err(e.into());
        }
    }
    Ok(Json(like_status(&pool, post_id, &key).await?))
}

/// DELETE /api/posts/:slug/like?liker_key=（亦接受 JSON body）→ {likes, liked: false}。
/// 未点赞过 → 幂等（DELETE 零行不报错，liked=false）
pub async fn unlike_post(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    Query(q): Query<LikeQuery>,
    body: ValidJson<LikeBody>,
) -> ApiResult<Json<LikeResult>> {
    // query 优先；body 可选（DELETE 允许空 body，解析失败/缺失时忽略，缺 key 统一 422）
    let raw = match q.liker_key {
        Some(k) => Some(k),
        None => match body {
            Ok(Json(b)) => Some(b.liker_key),
            Err(_) => None,
        },
    };
    let key = validate_liker_key(raw)?;
    let (pool, _db_type) = require_pool(&state).await?;
    let post_id = find_published_post(&pool, &slug).await?;

    sqlx::query("DELETE FROM post_likes WHERE post_id = ? AND liker_key = ?")
        .bind(post_id)
        .bind(&key)
        .execute(&pool)
        .await?;
    Ok(Json(like_status(&pool, post_id, &key).await?))
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
