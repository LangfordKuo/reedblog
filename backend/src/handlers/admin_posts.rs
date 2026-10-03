//! 管理接口：文章 CRUD（全部需要 Bearer）

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use sqlx::any::AnyRow;
use sqlx::{AnyConnection, AnyPool};
use sqlx::Row;

use crate::error::{ApiError, ApiResult, ValidJson};
use crate::models::{
    normalize_paging, AdminPostsQuery, Page, PostAdmin, PostBody, PostRevision,
    PostRevisionSummary, StickyBody,
};
use crate::pages::row_bool;
use crate::state::{now_rfc3339, require_pool, AppState};

use super::helpers::{
    check_auth, derive_excerpt, ensure_category_exists, fetch_tag_ids, is_unique_violation,
    last_insert_id_on, replace_post_tags, slug_taken, slugify, temp_slug,
};

// view_count / likes（契约「浏览量与点赞」条款）：后台只读展示，不做管理点赞；
// likes 为 post_likes 子查询计数（走 UNIQUE 索引最左前缀，后台列表分页量小可接受）
const ADMIN_COLUMNS: &str = "p.id, p.title, p.slug, p.content_md, p.excerpt, p.status, \
     p.category_id, c.name AS category_name, p.published_at, p.is_sticky, p.view_count, \
     (SELECT COUNT(*) FROM post_likes l WHERE l.post_id = p.id) AS likes, \
     p.created_at, p.updated_at";

const ADMIN_FROM: &str = "FROM posts p LEFT JOIN categories c ON c.id = p.category_id";

async fn row_to_post_admin(pool: &AnyPool, r: &AnyRow) -> ApiResult<PostAdmin> {
    let id = r.get::<i64, _>("id");
    Ok(PostAdmin {
        id,
        title: r.get::<String, _>("title"),
        slug: r.get::<String, _>("slug"),
        content_md: r.get::<String, _>("content_md"),
        excerpt: r.get::<String, _>("excerpt"),
        status: r.get::<String, _>("status"),
        category_id: r.try_get::<Option<i64>, _>("category_id").unwrap_or(None),
        category_name: r
            .try_get::<Option<String>, _>("category_name")
            .unwrap_or(None),
        tag_ids: fetch_tag_ids(pool, id).await?,
        published_at: r
            .try_get::<Option<String>, _>("published_at")
            .unwrap_or(None),
        is_sticky: row_bool(r, "is_sticky"),
        view_count: r.try_get::<i64, _>("view_count").unwrap_or(0),
        likes: r.try_get::<i64, _>("likes").unwrap_or(0),
        created_at: r.get::<String, _>("created_at"),
        updated_at: r.get::<String, _>("updated_at"),
    })
}

async fn load_post_admin(pool: &AnyPool, id: i64) -> ApiResult<Option<PostAdmin>> {
    let sql = format!("SELECT {ADMIN_COLUMNS} {ADMIN_FROM} WHERE p.id = ?");
    let row = sqlx::query(&sql).bind(id).fetch_optional(pool).await?;
    match row {
        None => Ok(None),
        Some(r) => Ok(Some(row_to_post_admin(pool, &r).await?)),
    }
}

// ---------- 文章修订历史（契约「文章修订历史」条款，2026-10-04 新增） ----------

/// 每篇文章最多保留的修订条数（契约写死的常量，不做配置项）
const MAX_REVISIONS_PER_POST: i64 = 20;

/// 文章存在性校验（契约「文章修订历史」：文章不存在 → 404）
async fn ensure_post_exists(pool: &AnyPool, id: i64) -> ApiResult<()> {
    let count: i64 = sqlx::query("SELECT COUNT(*) FROM posts WHERE id = ?")
        .bind(id)
        .fetch_one(pool)
        .await?
        .get(0);
    if count == 0 {
        return Err(ApiError::not_found());
    }
    Ok(())
}

/// 在调用方事务连接上插入一条修订，并裁剪到每篇 20 条上限。
/// 插入与裁剪都随调用方事务提交/回滚（避免写文章成功但快照丢失）。
async fn insert_revision(
    conn: &mut AnyConnection,
    post_id: i64,
    title: &str,
    content_md: &str,
    excerpt: &str,
    created_at: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO post_revisions (post_id, title, content_md, excerpt, created_at) \
         VALUES (?, ?, ?, ?, ?)",
    )
    .bind(post_id)
    .bind(title)
    .bind(content_md)
    .bind(excerpt)
    .bind(created_at)
    .execute(&mut *conn)
    .await?;

    // 裁剪：同 post_id 下按 id 从新到旧保留 20 条（id 自增，等价于时间序）。
    // 子查询用派生表包一层是 MySQL 的要求（不能在语句的子查询里直接读目标表），
    // SQLite 同样接受该写法；LIMIT 走绑定参数（双方言均可）。
    sqlx::query(
        "DELETE FROM post_revisions WHERE post_id = ? AND id NOT IN (\
             SELECT id FROM (\
                 SELECT id FROM post_revisions WHERE post_id = ? ORDER BY id DESC LIMIT ?\
             ) AS keep_ids\
         )",
    )
    .bind(post_id)
    .bind(post_id)
    .bind(MAX_REVISIONS_PER_POST)
    .execute(&mut *conn)
    .await?;

    Ok(())
}

/// 读取单条修订（含完整正文）；不存在或不属于该文章 → None（调用方映射 404）
async fn load_revision(pool: &AnyPool, post_id: i64, rev_id: i64) -> ApiResult<Option<PostRevision>> {
    let row = sqlx::query(
        "SELECT id, post_id, title, content_md, excerpt, created_at FROM post_revisions \
         WHERE id = ? AND post_id = ?",
    )
    .bind(rev_id)
    .bind(post_id)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|r| PostRevision {
        id: r.get("id"),
        post_id: r.get("post_id"),
        title: r.get("title"),
        content_md: r.get("content_md"),
        excerpt: r.get("excerpt"),
        created_at: r.get("created_at"),
    }))
}

fn validate_status(s: &str) -> ApiResult<()> {
    if s != "draft" && s != "published" && s != "scheduled" {
        return Err(ApiError::validation(
            "status 必须是 draft、published 或 scheduled",
        ));
    }
    Ok(())
}

/// 校验并归一化定时发布时间（契约「文章置顶与定时发布」条款）：
/// 必须是合法 RFC3339，且晚于当前时间（now 为 now_rfc3339() 产出，字典序即时间序）；
/// 通过后归一化为 UTC 秒精度（如 2026-10-03T12:00:00Z），保证与可见性比较的同格式。
/// 失败 → 422 validation_error。
fn normalize_scheduled_at(raw: &str, now: &str) -> ApiResult<String> {
    let dt = chrono::DateTime::parse_from_rfc3339(raw.trim()).map_err(|_| {
        ApiError::validation("published_at 必须是合法的 RFC3339 时间（如 2026-10-03T12:00:00Z）")
    })?;
    let normalized = dt
        .with_timezone(&chrono::Utc)
        .to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    if normalized.as_str() <= now {
        return Err(ApiError::validation(
            "定时发布时间必须晚于当前时间，请使用未来时间",
        ));
    }
    Ok(normalized)
}

fn slug_conflict(slug: &str) -> ApiError {
    ApiError::conflict("slug_taken", format!("slug '{slug}' 已被占用"))
}

/// GET /api/admin/posts?status=<draft|published|all>&page&per_page → 分页 [PostAdmin]，updated_at DESC
pub async fn admin_list_posts(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<AdminPostsQuery>,
) -> ApiResult<Json<Page<PostAdmin>>> {
    check_auth(&state, &headers).await?;
    let (pool, _db_type) = require_pool(&state).await?;
    let (page, per_page) = normalize_paging(q.page, q.per_page);

    let status = q.status.clone().unwrap_or_else(|| "all".to_string());
    let param: Option<String> = if status == "all" {
        None
    } else {
        validate_status(&status)?;
        Some(status)
    };
    let where_sql = if param.is_some() {
        "WHERE p.status = ?"
    } else {
        ""
    };

    let count_sql = format!("SELECT COUNT(*) FROM posts p {where_sql}");
    let mut cq = sqlx::query(&count_sql);
    if let Some(s) = &param {
        cq = cq.bind(s.clone());
    }
    let total: i64 = cq.fetch_one(&pool).await?.get(0);

    let list_sql = format!(
        "SELECT {ADMIN_COLUMNS} {ADMIN_FROM} {where_sql} \
         ORDER BY p.updated_at DESC LIMIT ? OFFSET ?"
    );
    let mut lq = sqlx::query(&list_sql);
    if let Some(s) = &param {
        lq = lq.bind(s.clone());
    }
    let rows = lq
        .bind(per_page)
        .bind((page - 1) * per_page)
        .fetch_all(&pool)
        .await?;

    let mut items = Vec::with_capacity(rows.len());
    for r in &rows {
        items.push(row_to_post_admin(&pool, r).await?);
    }
    Ok(Json(Page {
        items,
        total,
        page,
        per_page,
    }))
}

/// POST /api/admin/posts → 201 PostAdmin
pub async fn admin_create_post(
    State(state): State<AppState>,
    headers: HeaderMap,
    req_body: ValidJson<PostBody>,
) -> ApiResult<(StatusCode, Json<PostAdmin>)> {
    check_auth(&state, &headers).await?;
    let Json(body) = req_body.map_err(ApiError::from)?;
    let (pool, db_type) = require_pool(&state).await?;

    let title = body
        .title
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
        .ok_or_else(|| ApiError::validation("title 必填且不能为空"))?;
    let content_md = body
        .content_md
        .ok_or_else(|| ApiError::validation("content_md 必填"))?;
    let status = body
        .status
        .ok_or_else(|| ApiError::validation("status 必填"))?;
    validate_status(&status)?;

    let category_id = match body.category_id {
        Some(Some(cid)) => {
            ensure_category_exists(&pool, cid).await?;
            Some(cid)
        }
        _ => None,
    };

    let excerpt = body
        .excerpt
        .map(|e| e.trim().to_string())
        .filter(|e| !e.is_empty())
        .unwrap_or_else(|| derive_excerpt(&content_md));

    // slug：提供则原样使用；为空自动生成（ASCII slugify；纯中文回退 post-<id>，插入后回填）
    let provided_slug = body
        .slug
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let (mut slug, need_backfill) = match provided_slug {
        Some(s) => {
            if slug_taken(&pool, &s, None).await? {
                return Err(slug_conflict(&s));
            }
            (s, false)
        }
        None => {
            let gen = slugify(&title);
            if gen.is_empty() {
                (temp_slug(), true)
            } else {
                if slug_taken(&pool, &gen, None).await? {
                    return Err(slug_conflict(&gen));
                }
                (gen, false)
            }
        }
    };

    let now = now_rfc3339();
    // published_at：published 首次发布 → now；scheduled → 计划时间（必填、须为未来，
    // 归一化为 UTC 秒精度）；draft → None（契约「文章置顶与定时发布」条款）
    let published_at = match status.as_str() {
        "published" => Some(now.clone()),
        "scheduled" => {
            let raw = body
                .published_at
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .ok_or_else(|| {
                    ApiError::validation("定时发布必须提供发布时间 published_at（须为未来时间）")
                })?;
            Some(normalize_scheduled_at(raw, &now)?)
        }
        _ => None,
    };
    let is_sticky = body.is_sticky.unwrap_or(false);

    // 文章插入 +（纯中文标题时）slug 回填 + 初始修订同一事务（契约「文章修订历史」：
    // 创建即有 1 条初始修订；避免文章创建成功但修订丢失）
    let mut tx = pool.begin().await?;
    let insert = sqlx::query(
        "INSERT INTO posts (title, slug, excerpt, content_md, status, category_id, \
         published_at, is_sticky, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&title)
    .bind(&slug)
    .bind(&excerpt)
    .bind(&content_md)
    .bind(&status)
    .bind(category_id)
    .bind(published_at.as_deref())
    .bind(if is_sticky { 1i64 } else { 0i64 })
    .bind(&now)
    .bind(&now);
    if let Err(e) = insert.execute(&mut *tx).await {
        if is_unique_violation(&e) {
            return Err(slug_conflict(&slug));
        }
        return Err(e.into());
    }
    let id = last_insert_id_on(&mut *tx, &db_type).await?;
    if need_backfill {
        slug = format!("post-{id}");
        sqlx::query("UPDATE posts SET slug = ? WHERE id = ?")
            .bind(&slug)
            .bind(id)
            .execute(&mut *tx)
            .await?;
    }
    insert_revision(&mut *tx, id, &title, &content_md, &excerpt, &now).await?;
    tx.commit().await?;

    if let Some(tag_ids) = &body.tag_ids {
        replace_post_tags(&pool, id, tag_ids).await?;
    }

    let post = load_post_admin(&pool, id)
        .await?
        .ok_or_else(|| ApiError::internal("创建后读取文章失败"))?;

    // post.after_publish 通知钩子（扩展契约）：创建即发布时触发；返回值忽略、错误不阻断
    if status == "published" {
        if let Some(pa) = post.published_at.as_deref() {
            state
                .plugins()
                .run_post_after_publish(&post.title, &post.slug, pa)
                .await;
        }
    }

    Ok((StatusCode::CREATED, Json(post)))
}

/// GET /api/admin/posts/:id → PostAdmin
pub async fn admin_get_post(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<i64>,
) -> ApiResult<Json<PostAdmin>> {
    check_auth(&state, &headers).await?;
    let (pool, _db_type) = require_pool(&state).await?;
    load_post_admin(&pool, id)
        .await?
        .map(Json)
        .ok_or_else(ApiError::not_found)
}

/// PUT /api/admin/posts/:id → PostAdmin（字段可选更新）。
/// 状态转换与 published_at 规则（契约「文章置顶与定时发布」条款）：
/// - draft→published：published_at 为空则写入 now（现有行为）
/// - scheduled→published：允许，立即发布（published_at 改写为 now，触发 after_publish）
/// - published→scheduled：拒绝 → 422（先转草稿）
/// - scheduled→draft：允许，清空 published_at（取消定时）
/// - →scheduled（自 draft/scheduled）：published_at 必填且须为未来时间；
///   编辑已到点的 scheduled 文章（不触碰 status/published_at）不重新校验
pub async fn admin_update_post(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<i64>,
    req_body: ValidJson<PostBody>,
) -> ApiResult<Json<PostAdmin>> {
    check_auth(&state, &headers).await?;
    let Json(body) = req_body.map_err(ApiError::from)?;
    let (pool, _db_type) = require_pool(&state).await?;
    let existing = load_post_admin(&pool, id)
        .await?
        .ok_or_else(ApiError::not_found)?;

    let title = match body.title {
        Some(t) => {
            let t = t.trim().to_string();
            if t.is_empty() {
                return Err(ApiError::validation("title 不能为空"));
            }
            t
        }
        None => existing.title.clone(),
    };
    let content_md = body.content_md.unwrap_or(existing.content_md.clone());
    let excerpt = match body.excerpt {
        Some(e) => e.trim().to_string(),
        None => existing.excerpt.clone(),
    };
    let status = match body.status {
        Some(s) => {
            validate_status(&s)?;
            s
        }
        None => existing.status.clone(),
    };
    // published → scheduled 拒绝（契约「文章置顶与定时发布」条款：先转草稿再定时）
    if status == "scheduled" && existing.status == "published" {
        return Err(ApiError::validation(
            "已发布文章不能改为定时发布，请先转为草稿",
        ));
    }
    let category_id = match body.category_id {
        Some(Some(cid)) => {
            ensure_category_exists(&pool, cid).await?;
            Some(cid)
        }
        Some(None) => None, // 显式 null → 清空分类
        None => existing.category_id,
    };

    let mut slug = existing.slug.clone();
    if let Some(s) = body
        .slug
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
    {
        if s != slug {
            if slug_taken(&pool, &s, Some(id)).await? {
                return Err(slug_conflict(&s));
            }
            slug = s;
        }
    }

    let now = now_rfc3339();

    // published_at 解析（契约「文章置顶与定时发布」条款；body.published_at 仅
    // status=scheduled 时接受——显式提供即为「改期」，其余状态忽略该字段）
    let body_scheduled_at = body
        .published_at
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let entering_scheduled = status == "scheduled" && existing.status != "scheduled";
    let published_at = match status.as_str() {
        "scheduled" => {
            // 进入 scheduled 或显式改期 → 校验「必填 + 未来时间」并归一化；
            // 编辑已到点的 scheduled 文章（两者都没触碰）→ 保留原计划时间不重新校验，
            // 否则惰性到点可见后文章将永远无法编辑
            if entering_scheduled || body_scheduled_at.is_some() {
                let raw = body_scheduled_at
                    .or(existing.published_at.as_deref())
                    .ok_or_else(|| {
                        ApiError::validation(
                            "定时发布必须提供发布时间 published_at（须为未来时间）",
                        )
                    })?;
                Some(normalize_scheduled_at(raw, &now)?)
            } else {
                existing.published_at.clone()
            }
        }
        // scheduled→published 手动切换 = 立即发布（改写为 now）；
        // draft→published 为空则写入 now；其余（含重复保存 published）保持原值
        "published" => {
            if existing.status == "scheduled" || existing.published_at.is_none() {
                Some(now.clone())
            } else {
                existing.published_at.clone()
            }
        }
        // scheduled→draft = 取消定时，清空计划时间；published→draft 保持原值（现有行为）
        _ => {
            if existing.status == "scheduled" {
                None
            } else {
                existing.published_at.clone()
            }
        }
    };
    // 显式发布动作（触发 post.after_publish）：draft→published 首次发布 +
    // scheduled→published 手动立即发布；惰性到点自动可见不触发（无后台任务，取舍见契约）
    let became_published = status == "published"
        && existing.status != "published"
        && (existing.status == "scheduled" || existing.published_at.is_none());
    let is_sticky = body.is_sticky.unwrap_or(existing.is_sticky);

    // 内容变化判定（契约「文章修订历史」）：用 UPDATE 前已读到的当前行比对归一化后的
    // title/content_md/excerpt（不用 SQL 的 != 技巧）；未变化（如只改 status/category/tags）
    // 不新增修订，PATCH sticky 等操作根本不经过这里
    let content_changed = title != existing.title
        || content_md != existing.content_md
        || excerpt != existing.excerpt;

    // 更新与（内容变化时的）插图修订同一事务：避免更新成功但快照丢失
    let mut tx = pool.begin().await?;
    let update = sqlx::query(
        "UPDATE posts SET title = ?, slug = ?, excerpt = ?, content_md = ?, status = ?, \
         category_id = ?, published_at = ?, is_sticky = ?, updated_at = ? WHERE id = ?",
    )
    .bind(&title)
    .bind(&slug)
    .bind(&excerpt)
    .bind(&content_md)
    .bind(&status)
    .bind(category_id)
    .bind(published_at.as_deref())
    .bind(if is_sticky { 1i64 } else { 0i64 })
    .bind(&now)
    .bind(id);
    if let Err(e) = update.execute(&mut *tx).await {
        if is_unique_violation(&e) {
            return Err(slug_conflict(&slug));
        }
        return Err(e.into());
    }
    if content_changed {
        insert_revision(&mut *tx, id, &title, &content_md, &excerpt, &now).await?;
    }
    tx.commit().await?;

    if let Some(tag_ids) = &body.tag_ids {
        replace_post_tags(&pool, id, tag_ids).await?;
    }

    let post = load_post_admin(&pool, id)
        .await?
        .ok_or_else(|| ApiError::internal("更新后读取文章失败"))?;

    // post.after_publish 通知钩子（扩展契约）：显式发布动作时触发
    // （draft→published 首次发布、scheduled→published 手动立即发布）
    if became_published {
        if let Some(pa) = post.published_at.as_deref() {
            state
                .plugins()
                .run_post_after_publish(&post.title, &post.slug, pa)
                .await;
        }
    }

    Ok(Json(post))
}

/// PATCH /api/admin/posts/:id/sticky → PostAdmin（行内快捷置顶/取消置顶，
/// 契约「文章置顶与定时发布」条款；body `{is_sticky: bool}`，其余字段不变）
pub async fn admin_set_sticky(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<i64>,
    req_body: ValidJson<StickyBody>,
) -> ApiResult<Json<PostAdmin>> {
    check_auth(&state, &headers).await?;
    let Json(body) = req_body.map_err(ApiError::from)?;
    let (pool, _db_type) = require_pool(&state).await?;

    let count: i64 = sqlx::query("SELECT COUNT(*) FROM posts WHERE id = ?")
        .bind(id)
        .fetch_one(&pool)
        .await?
        .get(0);
    if count == 0 {
        return Err(ApiError::not_found());
    }

    let now = now_rfc3339();
    sqlx::query("UPDATE posts SET is_sticky = ?, updated_at = ? WHERE id = ?")
        .bind(if body.is_sticky { 1i64 } else { 0i64 })
        .bind(&now)
        .bind(id)
        .execute(&pool)
        .await?;

    let post = load_post_admin(&pool, id)
        .await?
        .ok_or_else(|| ApiError::internal("置顶切换后读取文章失败"))?;
    Ok(Json(post))
}

/// DELETE /api/admin/posts/:id → 204（连带清理标签关联、评论、点赞与修订历史）
pub async fn admin_delete_post(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<i64>,
) -> ApiResult<StatusCode> {
    check_auth(&state, &headers).await?;
    let (pool, _db_type) = require_pool(&state).await?;

    let count: i64 = sqlx::query("SELECT COUNT(*) FROM posts WHERE id = ?")
        .bind(id)
        .fetch_one(&pool)
        .await?
        .get(0);
    if count == 0 {
        return Err(ApiError::not_found());
    }

    sqlx::query("DELETE FROM post_tags WHERE post_id = ?")
        .bind(id)
        .execute(&pool)
        .await?;
    // comments.post_id 为通用目标 id（target_type 区分文章/页面），仅删文章评论行
    sqlx::query("DELETE FROM comments WHERE target_type = 'post' AND post_id = ?")
        .bind(id)
        .execute(&pool)
        .await?;
    sqlx::query("DELETE FROM post_likes WHERE post_id = ?")
        .bind(id)
        .execute(&pool)
        .await?;
    // 修订历史连带清理（契约「文章修订历史」：与点赞/评论清理同风格）
    sqlx::query("DELETE FROM post_revisions WHERE post_id = ?")
        .bind(id)
        .execute(&pool)
        .await?;
    sqlx::query("DELETE FROM posts WHERE id = ?")
        .bind(id)
        .execute(&pool)
        .await?;

    Ok(StatusCode::NO_CONTENT)
}

// ---------- 修订历史管理接口（契约「文章修订历史」条款，2026-10-04 新增） ----------

/// GET /api/admin/posts/:id/revisions → [PostRevisionSummary]
/// 按 created_at DESC, id DESC（时间戳秒精度，id 兜底确定性）；摘要不含正文
pub async fn admin_list_post_revisions(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<i64>,
) -> ApiResult<Json<Vec<PostRevisionSummary>>> {
    check_auth(&state, &headers).await?;
    let (pool, _db_type) = require_pool(&state).await?;
    ensure_post_exists(&pool, id).await?;

    let rows = sqlx::query(
        "SELECT id, post_id, title, content_md, created_at FROM post_revisions \
         WHERE post_id = ? ORDER BY created_at DESC, id DESC",
    )
    .bind(id)
    .fetch_all(&pool)
    .await?;

    Ok(Json(
        rows.iter()
            .map(|r| PostRevisionSummary {
                id: r.get("id"),
                post_id: r.get("post_id"),
                title: r.get("title"),
                // 正文字符数按 Unicode 字符计（MySQL CHAR_LENGTH/SQLite LENGTH 口径不一，
                // 放 Rust 侧统一）
                content_chars: r.get::<String, _>("content_md").chars().count() as i64,
                created_at: r.get("created_at"),
            })
            .collect(),
    ))
}

/// GET /api/admin/posts/:id/revisions/:rev_id → PostRevision（单条完整，含 content_md）
pub async fn admin_get_post_revision(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((id, rev_id)): Path<(i64, i64)>,
) -> ApiResult<Json<PostRevision>> {
    check_auth(&state, &headers).await?;
    let (pool, _db_type) = require_pool(&state).await?;
    ensure_post_exists(&pool, id).await?;
    load_revision(&pool, id, rev_id)
        .await?
        .map(Json)
        .ok_or_else(ApiError::not_found)
}

/// POST /api/admin/posts/:id/revisions/:rev_id/restore → PostAdmin
/// 把该修订的 title/content_md/excerpt 写回文章，并再插入一条新修订（回滚本身也留痕）；
/// 同一事务内完成；只动内容三字段 + updated_at，status/置顶/分类/标签/发布时间都不变
pub async fn admin_restore_post_revision(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((id, rev_id)): Path<(i64, i64)>,
) -> ApiResult<Json<PostAdmin>> {
    check_auth(&state, &headers).await?;
    let (pool, _db_type) = require_pool(&state).await?;
    ensure_post_exists(&pool, id).await?;
    let rev = load_revision(&pool, id, rev_id)
        .await?
        .ok_or_else(ApiError::not_found)?;

    let now = now_rfc3339();
    let mut tx = pool.begin().await?;
    sqlx::query(
        "UPDATE posts SET title = ?, content_md = ?, excerpt = ?, updated_at = ? WHERE id = ?",
    )
    .bind(&rev.title)
    .bind(&rev.content_md)
    .bind(&rev.excerpt)
    .bind(&now)
    .bind(id)
    .execute(&mut *tx)
    .await?;
    insert_revision(&mut *tx, id, &rev.title, &rev.content_md, &rev.excerpt, &now).await?;
    tx.commit().await?;

    let post = load_post_admin(&pool, id)
        .await?
        .ok_or_else(|| ApiError::internal("恢复修订后读取文章失败"))?;
    Ok(Json(post))
}
