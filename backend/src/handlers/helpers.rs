//! handler 共用的工具函数

use axum::http::{HeaderMap, StatusCode};
use rand::RngCore;
use sqlx::AnyPool;
use sqlx::Row;

use crate::auth::require_auth;
use crate::error::{ApiError, ApiResult};
use crate::models::{CategoryRef, CommentPub, CreateCommentRequest};
use crate::plugins::CommentDecision;
use crate::state::AppState;

/// 同一连接上取自增 id（re-export，命名更明确）
pub use crate::state::last_insert_id as last_insert_id_on;

/// 文章公开可见性条件（契约「文章置顶与定时发布」条款，惰性定时发布）：
/// published 恒可见；scheduled 到点（published_at <= :now）即可见。
/// 占位符必须绑定 `state::now_rfc3339()` 产出的 RFC3339 UTC 字符串——全库时间戳
/// 字典序即时间序，SQLite/MySQL 共用同一份 SQL；published_at 为 NULL 时比较结果
/// 为 NULL，该行自然排除。表别名固定为 `p`（所有公开/计数查询统一用 posts p）。
pub const VISIBLE_POST_SQL: &str =
    "(p.status = 'published' OR (p.status = 'scheduled' AND p.published_at <= ?))";

/// 管理接口统一鉴权：Bearer token 校验，失败 → 401 unauthorized
pub async fn check_auth(state: &AppState, headers: &HeaderMap) -> ApiResult<()> {
    let rt = state.runtime().await;
    require_auth(rt.jwt_secret, headers)?;
    Ok(())
}

/// ASCII slugify：小写、字母数字保留、其余折叠为 `-`、去首尾 `-`。
/// 纯中文/纯符号标题会得到空串，调用方需回退 `post-<id>`。
pub fn slugify(title: &str) -> String {
    let mut out = String::new();
    let mut pending_dash = false;
    for ch in title.chars() {
        if ch.is_ascii_alphanumeric() {
            if pending_dash && !out.is_empty() {
                out.push('-');
            }
            pending_dash = false;
            out.push(ch.to_ascii_lowercase());
        } else {
            pending_dash = true;
        }
    }
    out
}

/// 唯一性冲突极小概率的临时 slug（插入后回填 post-<id> 用）
pub fn temp_slug() -> String {
    let mut bytes = [0u8; 8];
    rand::thread_rng().fill_bytes(&mut bytes);
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    format!("tmp-{hex}")
}

/// excerpt 缺省时由正文生成纯文本摘要（契约「excerpt 为空时的回退」条款）：
/// 剥离 Markdown 语法（标题#、强调符、代码围栏及语言标记、行内代码反引号、
/// 表格分隔线与竖线、链接保留文字、图片语法、引用>、列表符号），
/// 代码块内容整体丢弃，多个空白折叠为单个空格，截断至 ≤200 字符（按 char，CJK 安全；
/// 截断时末位补省略号，总长仍 ≤200）。
pub fn derive_excerpt(content_md: &str) -> String {
    truncate_chars(&md_to_plain_text(content_md), 200)
}

/// Markdown → 纯文本（不截断）：derive_excerpt 与搜索 snippet 共用的同一条剥离逻辑
/// （契约「全文搜索」条款要求 snippet 用 derive_excerpt 同款规则，勿复制实现）。
pub fn md_to_plain_text(content_md: &str) -> String {
    let mut kept: Vec<String> = Vec::new();
    let mut in_code_block = false;
    for line in content_md.lines() {
        let trimmed = line.trim();
        // 代码围栏开/闭行（``` 及语言标记）本身不进摘要
        if trimmed.starts_with("```") {
            in_code_block = !in_code_block;
            continue;
        }
        // 代码块内容直接丢弃，不拼进摘要
        if in_code_block || trimmed.is_empty() {
            continue;
        }
        // 表格分隔线（|---|:--:|）与水平分割线（--- /*** / ___ / ===）整行丢弃
        if is_table_separator(trimmed) || is_horizontal_rule(trimmed) {
            continue;
        }
        let cleaned = clean_line(trimmed);
        if !cleaned.is_empty() {
            kept.push(cleaned);
        }
    }
    kept.join(" ")
        .split_whitespace()
        .filter(|tok| !is_separator_token(tok))
        .collect::<Vec<_>>()
        .join(" ")
}

// ---------- 全文搜索（契约「全文搜索」条款） ----------

/// 搜索词条数上限（q 按空白切分，多余词条忽略）
const MAX_SEARCH_TERMS: usize = 8;

/// snippet 窗口：命中点前 ≤40 字符、后 ≤60 字符（按 char 计，CJK 安全）
const SNIPPET_BEFORE: usize = 40;
const SNIPPET_AFTER: usize = 60;

/// 搜索分词：trim 后按空白切分，上限 8 个词条（多余忽略）
pub fn split_search_terms(q: &str) -> Vec<String> {
    q.split_whitespace()
        .take(MAX_SEARCH_TERMS)
        .map(|s| s.to_string())
        .collect()
}

/// LIKE 通配符转义：`\`、`%`、`_` 前补 `\`，配合 `ESCAPE '\'` 子句
/// （SQLite/MySQL 通用；防止用户输入 `%`/`_` 把搜索变成全匹配）
pub fn escape_like(term: &str) -> String {
    let mut out = String::with_capacity(term.len());
    for c in term.chars() {
        if matches!(c, '\\' | '%' | '_') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// 单字符小写化（保持 char 数 1:1，避免特殊字符小写展开导致下标错位）
fn lower_char(c: char) -> char {
    c.to_lowercase().next().unwrap_or(c)
}

/// 生成搜索 snippet：在 content_md 剥出的纯文本中定位任一词条的首个命中位置
/// （大小写不敏感），截取命中点前 ≤40、后 ≤60 字符的窗口；窗口两端非文本边界时补 `…`。
/// 纯文本中找不到命中（仅 title/excerpt 命中）→ 回退 fallback_excerpt。
/// 输出不含任何 HTML/Markdown markup，高亮由前端实现。
pub fn make_snippet(plain: &str, terms: &[String], fallback_excerpt: &str) -> String {
    let chars: Vec<char> = plain.chars().collect();
    let lower: Vec<char> = chars.iter().copied().map(lower_char).collect();

    // 任一词条的最早命中位置（词条自身也按小写比较）
    let mut hit: Option<(usize, usize)> = None;
    for term in terms {
        let t: Vec<char> = term.chars().map(lower_char).collect();
        if t.is_empty() || t.len() > lower.len() {
            continue;
        }
        if let Some(pos) = lower.windows(t.len()).position(|w| w == t.as_slice()) {
            if hit.map_or(true, |(s, _)| pos < s) {
                hit = Some((pos, pos + t.len()));
            }
        }
    }
    let Some((hit_start, hit_end)) = hit else {
        return fallback_excerpt.to_string();
    };

    let from = hit_start.saturating_sub(SNIPPET_BEFORE);
    let to = (hit_end + SNIPPET_AFTER).min(chars.len());
    let mut out = String::new();
    if from > 0 {
        out.push('…');
    }
    let window: String = chars[from..to].iter().collect();
    out.push_str(window.trim());
    if to < chars.len() {
        out.push('…');
    }
    out
}

/// 纯分隔符 token：全由 `-`/`=`/`:`/`|` 组成且长度 ≥2（单行化表格分隔线 |------| 去竖线后的残留）
fn is_separator_token(tok: &str) -> bool {
    tok.chars().count() >= 2 && tok.chars().all(|c| matches!(c, '-' | '=' | ':' | '|'))
}

/// 单行清洗：去引用/标题行首标记与列表符号，移除图片/链接语法，再剔除行内 Markdown 字符
fn clean_line(line: &str) -> String {
    // 行首引用 '>' 与标题 '#' 标记（可交错出现，如 "> ## 标题"）
    let s = line
        .trim_start_matches(|c: char| c == '>' || c == '#')
        .trim_start();
    let s = strip_list_marker(s);
    // 历史数据存在整篇被压成单行的情况，``` 围栏不再独占一行：行内成对剔除
    let s = strip_inline_fences(s);
    let s = strip_link_syntax(&s);
    let cleaned = strip_inline_marks(&s);
    cleaned.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// 剔除行内 ``` 围栏段（围栏符、语言标记与代码内容一并丢弃）；未闭合的丢到行尾
fn strip_inline_fences(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(start) = rest.find("```") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 3..];
        rest = match after.find("```") {
            Some(end) => &after[end + 3..],
            None => "",
        };
    }
    out.push_str(rest);
    out
}

/// 去掉行首列表符号：`- `、`* `、`+ `、`1. `、`2) `（符号后须为空白或行尾）
fn strip_list_marker(s: &str) -> &str {
    let b = s.as_bytes();
    match b.first() {
        Some(b'-') | Some(b'*') | Some(b'+') => {
            if b.len() == 1 {
                ""
            } else if b[1].is_ascii_whitespace() {
                s[1..].trim_start()
            } else {
                s
            }
        }
        Some(c) if c.is_ascii_digit() => {
            let mut j = 1;
            while j < b.len() && b[j].is_ascii_digit() {
                j += 1;
            }
            if j < b.len() && (b[j] == b'.' || b[j] == b')') {
                let k = j + 1;
                if k >= b.len() {
                    return "";
                }
                if b[k].is_ascii_whitespace() {
                    return s[k..].trim_start();
                }
            }
            s
        }
        _ => s,
    }
}

/// 图片 `![alt](url)` 整体移除；链接 `[文字](url)` / `[文字][ref]` 只保留文字
fn strip_link_syntax(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '!' && chars.get(i + 1) == Some(&'[') {
            if let Some((_, end)) = parse_link(&chars, i + 1) {
                i = end;
                continue;
            }
        }
        if chars[i] == '[' {
            if let Some((close, end)) = parse_link(&chars, i) {
                out.extend(&chars[i + 1..close]);
                i = end;
                continue;
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

/// 解析从 `open`（`[` 所在位置）开始的链接语法：返回（闭合 `]` 下标，整段语法结束下标·不含）。
/// `]` 之后（可隔空白）不是 `(` 或 `[` 时视为非链接语法，返回 None。
fn parse_link(chars: &[char], open: usize) -> Option<(usize, usize)> {
    let close = (open + 1..chars.len()).find(|&j| chars[j] == ']')?;
    let mut k = close + 1;
    while k < chars.len() && chars[k].is_whitespace() {
        k += 1;
    }
    match chars.get(k) {
        Some('(') => {
            let end = (k + 1..chars.len()).find(|&j| chars[j] == ')')?;
            Some((close, end + 1))
        }
        Some('[') => {
            let end = (k + 1..chars.len()).find(|&j| chars[j] == ']')?;
            Some((close, end + 1))
        }
        _ => None,
    }
}

/// 剔除行内 Markdown 字符：反引号、强调星号、裸方括号、残留 # 与 >；表格竖线换成空格。
/// 下划线仅在两侧均为字母数字时保留（snake_case 属正文），否则按强调符剔除。
fn strip_inline_marks(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::new();
    for (idx, &c) in chars.iter().enumerate() {
        match c {
            '|' => out.push(' '),
            '`' | '*' | '[' | ']' | '#' | '>' => {}
            '_' => {
                let prev_alnum = idx > 0 && chars[idx - 1].is_alphanumeric();
                let next_alnum = idx + 1 < chars.len() && chars[idx + 1].is_alphanumeric();
                if prev_alnum && next_alnum {
                    out.push('_');
                }
            }
            _ => out.push(c),
        }
    }
    out
}

/// 表格分隔线行：仅由 `|`、`-`、`:` 与空白组成，且至少各含一个 `|` 和 `-`
fn is_table_separator(line: &str) -> bool {
    let mut has_pipe = false;
    let mut has_dash = false;
    for c in line.chars() {
        match c {
            '|' => has_pipe = true,
            '-' => has_dash = true,
            ':' | ' ' | '\t' => {}
            _ => return false,
        }
    }
    has_pipe && has_dash
}

/// 水平分割线 / setext 标题下划线行：≥3 个相同的 `-`/`*`/`_`/`=`（可夹空白）
fn is_horizontal_rule(line: &str) -> bool {
    let cs: Vec<char> = line.chars().filter(|c| !c.is_whitespace()).collect();
    cs.len() >= 3 && cs.iter().all(|&c| c == cs[0]) && matches!(cs[0], '-' | '*' | '_' | '=')
}

/// 截断至 max 字符（按 char 计）；超长时取前 max-1 字符并补 `…`，总长仍 ≤max
fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let head: String = s.chars().take(max - 1).collect();
    format!("{}…", head.trim_end())
}

/// 旧版（有 bug 的）推导算法：仅压平空白、不剥 Markdown、截 200 字符。
/// 仅用于启动时识别历史脏数据，勿在新代码中调用。
fn legacy_flat_excerpt(content_md: &str) -> String {
    let flat = content_md.split_whitespace().collect::<Vec<_>>().join(" ");
    flat.chars().take(200).collect()
}

/// 一次性数据修复：旧算法把「压平空白的 Markdown 原文」当 excerpt 写库，
/// 这里按现行规则重新推导并回写。识别标准：excerpt 恰等于旧算法对该行 content_md
/// 的输出（用户手填摘要几乎不可能命中，不会误伤）。幂等：修复后不再命中。
pub async fn repair_legacy_excerpts(pool: &AnyPool) -> Result<(), sqlx::Error> {
    let rows = sqlx::query("SELECT id, excerpt, content_md FROM posts")
        .fetch_all(pool)
        .await?;
    for r in &rows {
        let excerpt = r.get::<String, _>("excerpt");
        let fixed = derive_excerpt(&r.get::<String, _>("content_md"));
        if excerpt != fixed && excerpt == legacy_flat_excerpt(&r.get::<String, _>("content_md")) {
            sqlx::query("UPDATE posts SET excerpt = ? WHERE id = ?")
                .bind(&fixed)
                .bind(r.get::<i64, _>("id"))
                .execute(pool)
                .await?;
        }
    }
    Ok(())
}

/// 查某篇文章的标签（公开形状 [{id, name}]，按名称排序）
pub async fn fetch_post_tags(pool: &AnyPool, post_id: i64) -> ApiResult<Vec<CategoryRef>> {
    let rows = sqlx::query(
        "SELECT t.id, t.name FROM tags t \
         JOIN post_tags pt ON pt.tag_id = t.id \
         WHERE pt.post_id = ? ORDER BY t.name",
    )
    .bind(post_id)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .iter()
        .map(|r| CategoryRef {
            id: r.get::<i64, _>("id"),
            name: r.get::<String, _>("name"),
        })
        .collect())
}

/// 查某篇文章的 tag_ids（管理形状，按 tag id 排序）
pub async fn fetch_tag_ids(pool: &AnyPool, post_id: i64) -> ApiResult<Vec<i64>> {
    let rows = sqlx::query("SELECT tag_id FROM post_tags WHERE post_id = ? ORDER BY tag_id")
        .bind(post_id)
        .fetch_all(pool)
        .await?;
    Ok(rows.iter().map(|r| r.get::<i64, _>("tag_id")).collect())
}

/// 全量替换文章的标签关联（忽略不存在的 tag_id）
pub async fn replace_post_tags(pool: &AnyPool, post_id: i64, tag_ids: &[i64]) -> ApiResult<()> {
    sqlx::query("DELETE FROM post_tags WHERE post_id = ?")
        .bind(post_id)
        .execute(pool)
        .await?;
    for &tid in tag_ids {
        let exists: i64 = sqlx::query("SELECT COUNT(*) FROM tags WHERE id = ?")
            .bind(tid)
            .fetch_one(pool)
            .await?
            .get::<i64, _>(0);
        if exists > 0 {
            sqlx::query("INSERT INTO post_tags (post_id, tag_id) VALUES (?, ?)")
                .bind(post_id)
                .bind(tid)
                .execute(pool)
                .await?;
        }
    }
    Ok(())
}

/// slug 是否已被占用（exclude_id 用于更新时排除自身）
pub async fn slug_taken(pool: &AnyPool, slug: &str, exclude_id: Option<i64>) -> ApiResult<bool> {
    let (sql, count) = match exclude_id {
        Some(id) => (
            "SELECT COUNT(*) FROM posts WHERE slug = ? AND id <> ?",
            Some(id),
        ),
        None => ("SELECT COUNT(*) FROM posts WHERE slug = ?", None),
    };
    let mut q = sqlx::query(sql).bind(slug);
    if let Some(id) = count {
        q = q.bind(id);
    }
    let n: i64 = q.fetch_one(pool).await?.get(0);
    Ok(n > 0)
}

/// 判断是否为唯一约束冲突（SQLite / MySQL 通用），用于并发下兜底
pub fn is_unique_violation(e: &sqlx::Error) -> bool {
    match e {
        sqlx::Error::Database(db_err) => {
            let code = db_err.code().unwrap_or_default();
            // SQLite: 2067 = SQLITE_CONSTRAINT_UNIQUE, 1555 = SQLITE_CONSTRAINT_PRIMARYKEY
            // MySQL: 23000 = integrity constraint violation（含 1062 duplicate entry）
            matches!(code.as_ref(), "2067" | "1555" | "23000")
        }
        _ => false,
    }
}

/// Markdown → HTML（文章渲染管线：before_render 改写 content_md 后由此渲染，
/// after_render 再改写结果 HTML；扩展契约「后端钩子」）
pub fn render_markdown(md: &str) -> String {
    use pulldown_cmark::{html, Options, Parser};
    let mut opts = Options::ENABLE_TABLES;
    opts.insert(Options::ENABLE_STRIKETHROUGH);
    opts.insert(Options::ENABLE_TASKLISTS);
    let parser = Parser::new_ext(md, opts);
    let mut out = String::new();
    html::push_html(&mut out, parser);
    out
}

/// 校验 category_id 存在（不存在 → 422 validation_error）
pub async fn ensure_category_exists(pool: &AnyPool, category_id: i64) -> ApiResult<()> {
    let n: i64 = sqlx::query("SELECT COUNT(*) FROM categories WHERE id = ?")
        .bind(category_id)
        .fetch_one(pool)
        .await?
        .get(0);
    if n == 0 {
        return Err(crate::error::ApiError::validation(
            "category_id 对应的分类不存在",
        ));
    }
    Ok(())
}

// ---------- 评论/回复共用管线（契约「评论回复」条款，2026-10-03 嵌套评论新增） ----------

/// 父评论校验 + 两级归一化的结果
#[derive(Debug, Default)]
pub struct ParentResolution {
    /// 顶级祖先（楼层）id；顶级评论为 None
    pub parent_id: Option<i64>,
    /// 被回复的中间楼层 id（仅「回复的回复」非 None）
    pub reply_to_id: Option<i64>,
    /// 被回复人作者名（响应冗余，与 reply_to_id 同生同灭）
    pub reply_to_name: Option<String>,
}

/// 校验并归一化 parent_id（文章评论与留言板留言同一套机制）：
/// - 父评论存在、与目标同 target（target_type + 目标 id 一致）、status=approved；
///   违规 → 422 validation_error（带明确 message）
/// - 两级归一化（Typecho/WP 风格）：父评论本身有 parent_id 时，新评论的 parent_id
///   改写为其顶级祖先 id，被回复的中间楼层保留在 reply_to_id（存储上永远两级）
///
/// 向上走链只是对异常数据的防御（写入侧已保证两级），深度超 64 视为数据异常 → 422。
pub async fn resolve_comment_parent(
    pool: &AnyPool,
    target_type: &str,
    target_id: i64,
    parent_id: Option<i64>,
) -> ApiResult<ParentResolution> {
    let Some(requested) = parent_id else {
        return Ok(ParentResolution::default());
    };
    if requested <= 0 {
        return Err(ApiError::validation("parent_id 非法"));
    }
    let row = sqlx::query(
        "SELECT id, post_id, target_type, status, author_name, parent_id \
         FROM comments WHERE id = ?",
    )
    .bind(requested)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| ApiError::validation("父评论不存在"))?;

    let row_target = row
        .try_get::<String, _>("target_type")
        .unwrap_or_else(|_| "post".to_string());
    if row_target != target_type || row.get::<i64, _>("post_id") != target_id {
        return Err(ApiError::validation("不能回复其他目标下的评论"));
    }
    if row.get::<String, _>("status") != "approved" {
        return Err(ApiError::validation("不能回复已隐藏的评论"));
    }

    let requested_author = row.get::<String, _>("author_name");
    let mut top_id = row.get::<i64, _>("id");
    let mut next = row.try_get::<Option<i64>, _>("parent_id").unwrap_or(None);
    let mut reply_to: Option<(i64, String)> = None;
    let mut depth = 0;
    while let Some(pid) = next {
        depth += 1;
        if depth > 64 {
            return Err(ApiError::validation("回复链过深（数据异常）"));
        }
        // 第一跳即被回复的中间楼层（其后若还有祖先属异常数据，只归一化不再记 reply_to）
        if reply_to.is_none() {
            reply_to = Some((top_id, requested_author.clone()));
        }
        top_id = pid;
        next = sqlx::query("SELECT parent_id FROM comments WHERE id = ?")
            .bind(pid)
            .fetch_optional(pool)
            .await?
            .and_then(|r| r.try_get::<Option<i64>, _>("parent_id").unwrap_or(None));
    }
    Ok(ParentResolution {
        parent_id: Some(top_id),
        reply_to_id: reply_to.as_ref().map(|(id, _)| *id),
        reply_to_name: reply_to.map(|(_, name)| name),
    })
}

/// POST 评论共用创建管线（文章 `/api/posts/:slug/comments` 与留言板
/// `/api/pages/:slug/comments` 两处调用，行为完全一致）：
/// 必填校验 → 父评论校验与两级归一化（422 先于钩子）→ comment.before_create 钩子链
/// （block → 403 comment_blocked；ctx 带归一化后的 parent_id/reply_to_id）→
/// 插入（先发后审：创建即 approved）→ 返回 CommentPub。
pub async fn create_comment_pipeline(
    state: &AppState,
    pool: &AnyPool,
    db_type: &str,
    target_type: &str,
    target_id: i64,
    slug: &str,
    req: CreateCommentRequest,
    blocked_default_reason: &str,
) -> ApiResult<CommentPub> {
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

    let parent = resolve_comment_parent(pool, target_type, target_id, req.parent_id).await?;

    let (author_name, email, content) = match state
        .plugins()
        .run_comment_before_create(
            slug,
            author_name,
            email.as_deref(),
            content,
            parent.parent_id,
            parent.reply_to_id,
        )
        .await
    {
        CommentDecision::Block { reason } => {
            let message = if reason.trim().is_empty() {
                blocked_default_reason.to_string()
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
    let author_name = author_name.trim().to_string();
    let content = content.trim().to_string();
    if author_name.is_empty() {
        return Err(ApiError::validation("author_name 不能为空"));
    }
    if content.is_empty() {
        return Err(ApiError::validation("content 不能为空"));
    }
    let email = email
        .map(|e| e.trim().to_string())
        .filter(|e| !e.is_empty());

    let created_at = crate::state::now_rfc3339();
    let mut conn = pool.acquire().await?;
    sqlx::query(
        "INSERT INTO comments (post_id, target_type, author_name, email, content, status, \
         parent_id, reply_to_id, created_at) \
         VALUES (?, ?, ?, ?, ?, 'approved', ?, ?, ?)",
    )
    .bind(target_id)
    .bind(target_type)
    .bind(&author_name)
    .bind(email.as_deref())
    .bind(&content)
    .bind(parent.parent_id)
    .bind(parent.reply_to_id)
    .bind(&created_at)
    .execute(&mut *conn)
    .await?;
    let id = last_insert_id_on(&mut conn, db_type).await?;

    Ok(CommentPub {
        id,
        author_name,
        content,
        created_at,
        parent_id: parent.parent_id,
        reply_to_id: parent.reply_to_id,
        reply_to_name: parent.reply_to_name,
    })
}

/// 公开评论列表 SELECT 列（文章/留言板共用）：rt 为 reply_to 的自 JOIN 别名
pub const PUBLIC_COMMENT_COLUMNS: &str = "c.id, c.author_name, c.content, c.created_at, \
     c.parent_id, c.reply_to_id, rt.author_name AS reply_to_name";

/// 公开评论列表 FROM 片段：LEFT JOIN 自身取被回复人作者名（reply_to_id 为 NULL 时同为 NULL）
pub const PUBLIC_COMMENT_FROM: &str =
    "FROM comments c LEFT JOIN comments rt ON rt.id = c.reply_to_id";

/// 隐藏线程过滤（契约「评论回复」条款）：parent_id 指向非 approved 评论的子回复
/// 一并排除（父恢复 approved 后线程整体重新可见；子回复自身 status 不连带变更）
pub const PUBLIC_COMMENT_THREAD_FILTER: &str = "(c.parent_id IS NULL OR EXISTS \
     (SELECT 1 FROM comments tp WHERE tp.id = c.parent_id AND tp.status = 'approved'))";

/// 公开评论列表行 → CommentPub（列集须含 PUBLIC_COMMENT_COLUMNS）
pub fn row_to_comment_pub(r: &sqlx::any::AnyRow) -> CommentPub {
    CommentPub {
        id: r.get::<i64, _>("id"),
        author_name: r.get::<String, _>("author_name"),
        content: r.get::<String, _>("content"),
        created_at: r.get::<String, _>("created_at"),
        parent_id: r.try_get::<Option<i64>, _>("parent_id").unwrap_or(None),
        reply_to_id: r.try_get::<Option<i64>, _>("reply_to_id").unwrap_or(None),
        reply_to_name: r
            .try_get::<Option<String>, _>("reply_to_name")
            .unwrap_or(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 契约场景：含标题/代码块/表格/链接/列表的 markdown 输入
    /// → 输出无 #、`、|、**、[] 等符号且 ≤200 字符
    #[test]
    fn strips_markdown_symbols() {
        let md = "## 标题一\n\
             \n\
             正文第一段，**加粗**与`行内代码`，还有[链接文字](https://example.com/a)。\n\
             \n\
             ```rust\n\
             fn main() {\n\
             \x20    println!(\"secret_code_body\");\n\
             }\n\
             ```\n\
             \n\
             | 列A | 列B |\n\
             |------|:----:|\n\
             | 单元1 | 单元2 |\n\
             \n\
             - 列表项一\n\
             - 列表项二\n\
             \n\
             > 引用块内容\n\
             \n\
             ![图片alt](img.png)\n";
        let out = derive_excerpt(md);
        for sym in ['#', '`', '|', '*', '[', ']'] {
            assert!(!out.contains(sym), "摘要不应包含 {sym}：{out}");
        }
        assert!(!out.contains("**"), "摘要不应包含 **：{out}");
        assert!(out.chars().count() <= 200, "摘要应 ≤200 字符：{out}");
        // 代码块内容直接丢弃（含语言标记与代码正文）
        assert!(!out.contains("secret_code_body"), "{out}");
        assert!(!out.contains("println"), "{out}");
        assert!(!out.contains("rust"), "{out}");
        // 链接保留文字、丢弃 URL；图片语法整体移除
        assert!(out.contains("链接文字"), "{out}");
        assert!(!out.contains("example.com"), "{out}");
        assert!(!out.contains("img.png"), "{out}");
        // 标题、表格单元格、列表、引用的文字保留
        assert!(out.contains("标题一"), "{out}");
        assert!(out.contains("单元1"), "{out}");
        assert!(out.contains("列表项一"), "{out}");
        assert!(out.contains("引用块内容"), "{out}");
        assert!(!out.contains('>'), "{out}");
        // 表格分隔线/水平线不混入摘要
        assert!(!out.contains("---"), "{out}");
        // 多个空白折叠为单个空格
        assert!(!out.contains("  "), "{out}");
    }

    #[test]
    fn emphasis_stripped_but_snake_case_kept() {
        assert_eq!(
            derive_excerpt("**粗体** _斜体_ __重粗__ use_snake_case"),
            "粗体 斜体 重粗 use_snake_case"
        );
    }

    #[test]
    fn headings_quotes_and_ordered_lists() {
        assert_eq!(derive_excerpt("> ## 引用里的标题"), "引用里的标题");
        assert_eq!(derive_excerpt("1. 第一\n2) 第二"), "第一 第二");
        // 行内 # 也剔除，避免残留标题符
        assert!(!derive_excerpt("C# 与 #tag").contains('#'));
    }

    #[test]
    fn table_lines_flatten_to_text() {
        let md = "# 标题\n\n| a | b |\n|---|---|\n| 1 | 2 |";
        assert_eq!(derive_excerpt(md), "标题 a b 1 2");
    }

    #[test]
    fn code_block_content_dropped() {
        assert_eq!(derive_excerpt("```js\nconsole.log(1);\n```"), "");
        // 未闭合围栏：其余内容全部丢弃
        assert_eq!(derive_excerpt("开头\n```\ncode"), "开头");
        // 行内代码只去反引号、保留内容
        assert_eq!(
            derive_excerpt("用 `cargo build` 命令"),
            "用 cargo build 命令"
        );
    }

    #[test]
    fn plain_text_and_empty_passthrough() {
        assert_eq!(derive_excerpt("draft body"), "draft body");
        assert_eq!(derive_excerpt(""), "");
        assert_eq!(derive_excerpt("```\n```"), "");
    }

    #[test]
    fn truncates_to_200_chars_with_ellipsis() {
        assert_eq!(derive_excerpt(&"字".repeat(500)).chars().count(), 200);
        assert!(derive_excerpt(&"字".repeat(500)).ends_with('…'));
        // 恰好 200 字符不截断、不加省略号
        let exact = "a".repeat(200);
        assert_eq!(derive_excerpt(&exact), exact);
        // 201 字符 → 截断后仍 ≤200
        assert_eq!(derive_excerpt(&"a".repeat(201)).chars().count(), 200);
    }

    #[test]
    fn whitespace_collapsed() {
        assert_eq!(
            derive_excerpt("第一段\n\n\n第二段\t\t更多   空格"),
            "第一段 第二段 更多 空格"
        );
    }

    #[test]
    fn flattened_single_line_content() {
        // 历史脏数据：整篇被压成单行，围栏/表格/标题/引用全部内联
        let md = "## 标题Axum 很顺手。```rustfn main() { println!(\"drop_me\"); }```## 二段\
             支持 `Any` 驱动：| 特性 | SQLite ||------|--------|| 部署 | 单文件 |- [x] 迁移> 轻量";
        let out = derive_excerpt(md);
        // 行内围栏中的代码同样直接丢弃
        assert!(!out.contains("drop_me"), "{out}");
        assert!(!out.contains("println"), "{out}");
        assert!(!out.contains("rust"), "{out}");
        for sym in ['#', '`', '|', '*', '[', ']', '>'] {
            assert!(!out.contains(sym), "摘要不应包含 {sym}：{out}");
        }
        // 内联表格分隔线残留的破折号串被过滤
        assert!(!out.contains("---"), "{out}");
        assert!(out.contains("标题"), "{out}");
        assert!(out.contains("二段"), "{out}");
        assert!(out.contains("特性 SQLite"), "{out}");
        assert!(out.chars().count() <= 200, "{out}");
    }

    #[test]
    fn legacy_detection_matches_old_buggy_output() {
        // 启动修复的识别标准：旧算法输出 == 库中脏 excerpt
        let md = "## T\n正文 **加粗**";
        assert_eq!(legacy_flat_excerpt(md), "## T 正文 **加粗**");
        assert_eq!(derive_excerpt(md), "T 正文 加粗");
        // 纯文本内容两者一致（修复为无害重写，条件 excerpt != fixed 会跳过）
        assert_eq!(
            legacy_flat_excerpt("draft body"),
            derive_excerpt("draft body")
        );
    }

    // ---------- 全文搜索工具 ----------

    #[test]
    fn escape_like_escapes_wildcards() {
        assert_eq!(escape_like("100%"), r"100\%");
        assert_eq!(escape_like("a_b"), r"a\_b");
        assert_eq!(escape_like(r"c:\path"), r"c:\\path");
        assert_eq!(escape_like("%_\\"), r"\%\_\\");
        // 普通词条原样保留
        assert_eq!(escape_like("rust 入门"), "rust 入门");
        assert_eq!(escape_like(""), "");
    }

    #[test]
    fn split_terms_trims_and_caps_at_eight() {
        assert_eq!(split_search_terms("  rust   axum "), ["rust", "axum"]);
        assert!(split_search_terms("   ").is_empty());
        assert!(split_search_terms("").is_empty());
        let many: Vec<String> = (0..12).map(|i| format!("t{i}")).collect();
        let terms = split_search_terms(&many.join(" "));
        assert_eq!(terms.len(), 8);
        assert_eq!(terms[0], "t0");
        assert_eq!(terms[7], "t7");
    }

    #[test]
    fn snippet_window_truncates_with_ellipsis() {
        // 命中点前后都超长 → 两端 `…`，前 ≤40 后 ≤60（按 char，CJK 安全）
        let plain = format!("{}命中{}尾", "前".repeat(100), "后".repeat(100));
        let s = make_snippet(&plain, &["命中".to_string()], "回退摘要");
        assert!(s.starts_with('…'), "{s}");
        assert!(s.ends_with('…'), "{s}");
        assert_eq!(s.chars().count(), 1 + 40 + 2 + 60 + 1);
        assert!(s.contains("命中"));
        assert!(!s.contains("回退摘要"));
    }

    #[test]
    fn snippet_short_text_no_ellipsis_and_case_insensitive() {
        // 文本不足窗口 → 原样返回、不加 `…`；命中大小写不敏感、保留原文大小写
        assert_eq!(
            make_snippet("学习 Rust 很有趣", &["RUST".to_string()], "fb"),
            "学习 Rust 很有趣"
        );
        assert_eq!(
            make_snippet("Hello World", &["hello".to_string()], "fb"),
            "Hello World"
        );
    }

    #[test]
    fn snippet_first_hit_among_multiple_terms() {
        // 多词条命中时取文本中位置最早的命中点
        let plain = "alpha one beta two";
        let s = make_snippet(plain, &["two".to_string(), "one".to_string()], "fb");
        assert!(s.contains("one"));
        assert_eq!(s, plain); // 短文本整段返回
    }

    #[test]
    fn snippet_falls_back_to_excerpt_when_not_in_plain_text() {
        // 词条仅命中 title/excerpt，纯文本中找不到 → 回退 excerpt
        let s = make_snippet("body text only", &["标题词".to_string()], "这是回退摘要");
        assert_eq!(s, "这是回退摘要");
        let s = make_snippet("", &["anything".to_string()], "摘要B");
        assert_eq!(s, "摘要B");
    }

    #[test]
    fn snippet_contains_no_markup() {
        // snippet 基于剥净的纯文本：markdown 符号不得出现
        let md = "## 标题\n\n**加粗**内容 `code` 与 needle 在此";
        let plain = md_to_plain_text(md);
        let s = make_snippet(&plain, &["needle".to_string()], "fb");
        assert!(s.contains("needle"));
        for sym in ['#', '*', '`'] {
            assert!(!s.contains(sym), "snippet 不应包含 {sym}：{s}");
        }
    }

    #[test]
    fn plain_text_shared_with_derive_excerpt() {
        // derive_excerpt = md_to_plain_text + 200 截断（同一逻辑，无复制粘贴）
        let md = "# 标题\n\n正文 **加粗**";
        assert_eq!(md_to_plain_text(md), "标题 正文 加粗");
        assert_eq!(derive_excerpt(md), md_to_plain_text(md));
        // 超 200 才截断
        let long = "字".repeat(300);
        assert_eq!(md_to_plain_text(&long).chars().count(), 300);
        assert_eq!(derive_excerpt(&long).chars().count(), 200);
    }
}
