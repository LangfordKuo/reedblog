//! handler 共用的工具函数

use axum::http::HeaderMap;
use rand::RngCore;
use sqlx::AnyPool;
use sqlx::Row;

use crate::auth::require_auth;
use crate::error::ApiResult;
use crate::models::CategoryRef;
use crate::state::AppState;

/// 同一连接上取自增 id（re-export，命名更明确）
pub use crate::state::last_insert_id as last_insert_id_on;

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
    let flat = kept
        .join(" ")
        .split_whitespace()
        .filter(|tok| !is_separator_token(tok))
        .collect::<Vec<_>>()
        .join(" ");
    truncate_chars(&flat, 200)
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
    let rows =
        sqlx::query("SELECT tag_id FROM post_tags WHERE post_id = ? ORDER BY tag_id")
            .bind(post_id)
            .fetch_all(pool)
            .await?;
    Ok(rows.iter().map(|r| r.get::<i64, _>("tag_id")).collect())
}

/// 全量替换文章的标签关联（忽略不存在的 tag_id）
pub async fn replace_post_tags(
    pool: &AnyPool,
    post_id: i64,
    tag_ids: &[i64],
) -> ApiResult<()> {
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
pub async fn slug_taken(
    pool: &AnyPool,
    slug: &str,
    exclude_id: Option<i64>,
) -> ApiResult<bool> {
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
        assert_eq!(derive_excerpt("用 `cargo build` 命令"), "用 cargo build 命令");
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
        assert_eq!(legacy_flat_excerpt("draft body"), derive_excerpt("draft body"));
    }
}
