//! RSS 2.0 feed 与 sitemap.xml（契约「RSS 与 sitemap」条款；已安装后公开）：
//! - GET /api/feed.xml    → 最新 20 篇 published 文章，RFC 822 pubDate，全文本 XML 转义
//! - GET /api/sitemap.xml → 首页 + 全部 published 文章（lastmod=updated_at）+ 标签/分类/归档索引
//!
//! 站点绝对 URL：config.toml [server] base_url 非空优先（去尾 /）；
//! 为空从请求头推导（X-Forwarded-Proto/X-Forwarded-Host 优先，反代场景；再回退 Host）。

use axum::extract::State;
use axum::http::{header, HeaderMap};
use axum::response::{IntoResponse, Response};
use sqlx::Row;

use crate::error::ApiResult;
use crate::state::{require_pool, AppState};

use super::helpers::derive_excerpt;

/// feed 中最多输出的文章数
const FEED_POST_LIMIT: i64 = 20;

/// XML 文本转义（& < > " '），feed/sitemap 的所有插值都必须经过它
pub fn xml_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(c),
        }
    }
    out
}

/// RFC3339（库中时间戳格式）→ RFC 822（RSS pubDate，如 "Sat, 03 Oct 2026 12:00:00 +0000"）；
/// 解析失败返回空串（调用方跳过该元素）。手动格式化而非 to_rfc2822：后者日不补零。
fn rfc3339_to_rfc822(ts: &str) -> String {
    chrono::DateTime::parse_from_rfc3339(ts)
        .map(|dt| {
            dt.with_timezone(&chrono::Utc)
                .format("%a, %d %b %Y %H:%M:%S %z")
                .to_string()
        })
        .unwrap_or_default()
}

/// 请求头取值（trim 后非空才算）
fn header_value(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)?
        .to_str()
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// 站点绝对 URL（无尾斜杠）。base_url 配置优先；否则按请求头推导：
/// scheme 仅接受 http/https（防头注入），host 取 X-Forwarded-Host 首个值 → Host。
pub fn site_base_url(state: &AppState, headers: &HeaderMap) -> String {
    let configured = state.configured_base_url();
    let configured = configured.trim().trim_end_matches('/');
    if !configured.is_empty() {
        return configured.to_string();
    }
    let scheme = header_value(headers, "x-forwarded-proto")
        .and_then(|v| v.split(',').next().map(|s| s.trim().to_ascii_lowercase()))
        .filter(|s| s == "http" || s == "https")
        .unwrap_or_else(|| "http".to_string());
    let host = header_value(headers, "x-forwarded-host")
        .and_then(|v| {
            v.split(',')
                .next()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
        })
        .or_else(|| header_value(headers, "host"))
        .unwrap_or_else(|| "localhost".to_string());
    format!("{scheme}://{host}")
}

/// 文章前台绝对 URL（与前端路由 /posts/:slug 一致；slug 百分号编码）
fn post_url(base: &str, slug: &str) -> String {
    format!("{base}/posts/{}", urlencoding::encode(slug))
}

/// GET /api/feed.xml → RSS 2.0（application/rss+xml; charset=utf-8）
pub async fn feed_xml(State(state): State<AppState>, headers: HeaderMap) -> ApiResult<Response> {
    let (pool, _db_type) = require_pool(&state).await?;
    let base = site_base_url(&state, &headers);
    let rt = state.runtime().await;

    let rows = sqlx::query(
        "SELECT title, slug, excerpt, content_md, published_at FROM posts \
         WHERE status = 'published' ORDER BY published_at DESC LIMIT ?",
    )
    .bind(FEED_POST_LIMIT)
    .fetch_all(&pool)
    .await?;

    let description = if rt.site_subtitle.trim().is_empty() {
        rt.site_title.clone()
    } else {
        rt.site_subtitle.clone()
    };

    let mut xml = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<rss version=\"2.0\">\n  <channel>\n");
    xml.push_str(&format!("    <title>{}</title>\n", xml_escape(&rt.site_title)));
    xml.push_str(&format!("    <link>{}</link>\n", xml_escape(&base)));
    xml.push_str(&format!(
        "    <description>{}</description>\n",
        xml_escape(&description)
    ));
    for r in &rows {
        let slug = r.get::<String, _>("slug");
        let link = post_url(&base, &slug);
        // excerpt 为空时的回退与公开列表一致（契约「excerpt 回退」条款）
        let excerpt = r.get::<String, _>("excerpt");
        let excerpt = if excerpt.trim().is_empty() {
            derive_excerpt(&r.try_get::<String, _>("content_md").unwrap_or_default())
        } else {
            excerpt
        };
        let pub_date = rfc3339_to_rfc822(
            &r.try_get::<Option<String>, _>("published_at")
                .unwrap_or(None)
                .unwrap_or_default(),
        );

        xml.push_str("    <item>\n");
        xml.push_str(&format!(
            "      <title>{}</title>\n",
            xml_escape(&r.get::<String, _>("title"))
        ));
        xml.push_str(&format!("      <link>{}</link>\n", xml_escape(&link)));
        xml.push_str(&format!(
            "      <guid isPermaLink=\"true\">{}</guid>\n",
            xml_escape(&link)
        ));
        if !pub_date.is_empty() {
            xml.push_str(&format!("      <pubDate>{pub_date}</pubDate>\n"));
        }
        xml.push_str(&format!(
            "      <description>{}</description>\n",
            xml_escape(&excerpt)
        ));
        xml.push_str("    </item>\n");
    }
    xml.push_str("  </channel>\n</rss>\n");

    Ok((
        [(header::CONTENT_TYPE, "application/rss+xml; charset=utf-8")],
        xml,
    )
        .into_response())
}

/// GET /api/sitemap.xml → urlset（application/xml; charset=utf-8）
/// 首页 + 全部 published 文章（lastmod=updated_at，W3C datetime）+ 标签/分类/归档索引
pub async fn sitemap_xml(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> ApiResult<Response> {
    let (pool, _db_type) = require_pool(&state).await?;
    let base = site_base_url(&state, &headers);

    let rows = sqlx::query(
        "SELECT slug, updated_at FROM posts \
         WHERE status = 'published' ORDER BY published_at DESC",
    )
    .fetch_all(&pool)
    .await?;

    // (loc, lastmod)；固定页无 lastmod
    let mut urls: Vec<(String, Option<String>)> = vec![(format!("{base}/"), None)];
    for r in &rows {
        let slug = r.get::<String, _>("slug");
        let lastmod = r
            .try_get::<Option<String>, _>("updated_at")
            .unwrap_or(None)
            .filter(|s| !s.trim().is_empty());
        urls.push((post_url(&base, &slug), lastmod));
    }
    urls.push((format!("{base}/tags"), None));
    urls.push((format!("{base}/categories"), None));
    urls.push((format!("{base}/archive"), None));

    let mut xml = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <urlset xmlns=\"http://www.sitemaps.org/schemas/sitemap/0.9\">\n",
    );
    for (loc, lastmod) in urls {
        xml.push_str("  <url>\n");
        xml.push_str(&format!("    <loc>{}</loc>\n", xml_escape(&loc)));
        if let Some(lm) = lastmod {
            xml.push_str(&format!("    <lastmod>{}</lastmod>\n", xml_escape(&lm)));
        }
        xml.push_str("  </url>\n");
    }
    xml.push_str("</urlset>\n");

    Ok(([(header::CONTENT_TYPE, "application/xml; charset=utf-8")], xml).into_response())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_xml_special_chars() {
        assert_eq!(
            xml_escape("A & B < C > D \" E ' F"),
            "A &amp; B &lt; C &gt; D &quot; E &apos; F"
        );
        // 已转义串再转义不会破坏（& 变 &amp;amp; 属预期行为，不做二次识别）
        assert_eq!(xml_escape("&amp;"), "&amp;amp;");
        assert_eq!(xml_escape("中文标题"), "中文标题");
    }

    #[test]
    fn converts_rfc3339_to_rfc822() {
        assert_eq!(
            rfc3339_to_rfc822("2026-10-03T12:00:00Z"),
            "Sat, 03 Oct 2026 12:00:00 +0000"
        );
        // 非法/空时间戳 → 空串（item 省略 pubDate）
        assert_eq!(rfc3339_to_rfc822(""), "");
        assert_eq!(rfc3339_to_rfc822("not-a-date"), "");
    }

    #[test]
    fn post_url_matches_frontend_route() {
        assert_eq!(
            post_url("https://blog.example.com", "hello-world"),
            "https://blog.example.com/posts/hello-world"
        );
        // 非 ASCII slug 百分号编码
        assert_eq!(
            post_url("http://x.dev", "你好"),
            "http://x.dev/posts/%E4%BD%A0%E5%A5%BD"
        );
    }
}
