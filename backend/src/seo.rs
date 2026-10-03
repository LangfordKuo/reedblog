//! SEO / 分享元信息纯逻辑（契约「SEO / 分享元信息」条款，2026-10-04 新增）：
//! 爬虫/社媒预览 UA 白名单、HTML 转义、Markdown 首图提取、最小 OG HTML 与 robots.txt 渲染、
//! JSON-LD 构造。HTTP 路由见 `handlers::seo`（**非 /api** 顶层路由，Nginx 按 UA 分流）。
//!
//! 硬性约束：手写字符串拼接（不引新依赖）；所有插值一律经 [`escape_html`] 或 JSON-LD
//! 序列化（`<`/`>`/`&` 转 unicode 转义），标题/描述/正文文本绝不原样进入 HTML。

/// 爬虫 / 社媒预览 UA 白名单（大小写不敏感子串匹配；契约「SEO / 分享元信息」）。
/// 与 `views::is_bot_ua` 的宽口径（浏览量不计数，含 curl/wget/python 等工具）刻意区分：
/// 这里只认搜索引擎与社媒预览客户端，普通命令行工具不应拿到 OG HTML（未命中走 302）。
/// 前三个关键字已覆盖 googlebot/bingbot/twitterbot/slackbot/telegrambot/discordbot/baiduspider，
/// 其余逐项列出以便与契约白名单一一对应。
const CRAWLER_UA_KEYWORDS: [&str; 15] = [
    "bot",
    "crawler",
    "spider",
    "facebookexternalhit",
    "twitterbot",
    "slackbot",
    "telegrambot",
    "whatsapp",
    "discordbot",
    "googlebot",
    "bingbot",
    "bingpreview",
    "baiduspider",
    "micromessenger",
    "wechat",
];

/// 是否命中爬虫 / 社媒预览白名单
pub fn is_crawler_ua(ua: &str) -> bool {
    let lower = ua.to_ascii_lowercase();
    CRAWLER_UA_KEYWORDS.iter().any(|k| lower.contains(k))
}

/// HTML 文本/属性转义（`& < > " '`）——OG HTML 的所有插值必须经过它
pub fn escape_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

/// 从 content_md 取正文第一张 Markdown 图片 URL（跳过 ``` / ~~~ 代码围栏内），无则 None
pub fn first_image_url(content_md: &str) -> Option<String> {
    let mut in_fence = false;
    for line in content_md.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            in_fence = !in_fence;
            continue;
        }
        if in_fence {
            continue;
        }
        if let Some(url) = find_image_url(line) {
            return Some(url);
        }
    }
    None
}

/// 单行内首个 `![alt](url)` 的 URL；支持 `<url>` 包裹与 `"title"` 后缀。
/// 只按字节扫描 ASCII 标记（`!`/`[`/`]`/`(`/`)`），UTF-8 边界安全
fn find_image_url(line: &str) -> Option<String> {
    let bytes = line.as_bytes();
    let mut i = 0usize;
    while i + 1 < bytes.len() {
        if bytes[i] == b'!' && bytes[i + 1] == b'[' {
            if let Some(bracket_rel) = line[i + 2..].find(']') {
                let after = i + 2 + bracket_rel + 1;
                if let Some(paren_rest) = line[after..].strip_prefix('(') {
                    let inner = match paren_rest.find(')') {
                        Some(end) => &paren_rest[..end],
                        None => paren_rest,
                    }
                    .trim();
                    let url = if let Some(rest) = inner.strip_prefix('<') {
                        rest.split('>').next().unwrap_or("").trim()
                    } else {
                        inner.split_whitespace().next().unwrap_or("")
                    };
                    if !url.is_empty() {
                        return Some(url.to_string());
                    }
                }
            }
        }
        i += 1;
    }
    None
}

/// 相对 URL 补绝对（基于站点 base URL，无尾 `/`）；已是绝对地址时原样规范化。
/// base 非法 / URL 畸形 → None（调用方省略该标签）
pub fn absolutize(base: &str, url: &str) -> Option<String> {
    let url = url.trim();
    if url.is_empty() {
        return None;
    }
    url::Url::parse(base)
        .ok()?
        .join(url)
        .ok()
        .map(|u| u.to_string())
}

/// 纯文本按非空白字符数的字数估算（JSON-LD wordCount；CJK 场景下近似「字」）
pub fn word_count(plain: &str) -> usize {
    plain.chars().filter(|c| !c.is_whitespace()).count()
}

/// 最小 OG HTML 渲染入参
pub struct OgPage<'a> {
    pub title: &'a str,
    pub description: &'a str,
    pub canonical: &'a str,
    /// 站点根（无尾 `/`；RSS 自动发现链接 `{base}/api/feed.xml` 用）
    pub base: &'a str,
    pub site_name: &'a str,
    /// `article`（文章）/ `website`（页面）
    pub og_type: &'a str,
    /// 已绝对化的分享图；None 时省略 og:image/twitter:image 且 twitter:card=summary
    pub image: Option<&'a str>,
    pub json_ld: &'a serde_json::Value,
}

/// 渲染最小 OG HTML（仅 head 元信息 + 一段指向原文的正文，不放正文内容避免重复内容）
pub fn render_og_html(p: &OgPage<'_>) -> String {
    let site = p.site_name.trim();
    let full_title = if site.is_empty() || site == p.title {
        p.title.to_string()
    } else {
        format!("{} - {}", p.title, site)
    };
    let has_desc = !p.description.trim().is_empty();

    let mut h = String::with_capacity(1024);
    h.push_str("<!doctype html>\n<html lang=\"zh-CN\">\n<head>\n");
    h.push_str("<meta charset=\"utf-8\">\n");
    h.push_str("<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n");
    h.push_str(&format!("<title>{}</title>\n", escape_html(&full_title)));
    if has_desc {
        h.push_str(&format!(
            "<meta name=\"description\" content=\"{}\">\n",
            escape_html(p.description)
        ));
    }
    h.push_str(&format!(
        "<link rel=\"canonical\" href=\"{}\">\n",
        escape_html(p.canonical)
    ));
    // RSS 自动发现（契约「SEO / 分享元信息」）：非 JS 爬虫也能在 OG HTML 里发现 feed
    h.push_str(&format!(
        "<link rel=\"alternate\" type=\"application/rss+xml\" href=\"{}/api/feed.xml\">\n",
        escape_html(p.base.trim_end_matches('/'))
    ));
    h.push_str(&format!(
        "<meta property=\"og:type\" content=\"{}\">\n",
        escape_html(p.og_type)
    ));
    h.push_str(&format!(
        "<meta property=\"og:title\" content=\"{}\">\n",
        escape_html(p.title)
    ));
    if has_desc {
        h.push_str(&format!(
            "<meta property=\"og:description\" content=\"{}\">\n",
            escape_html(p.description)
        ));
    }
    h.push_str(&format!(
        "<meta property=\"og:url\" content=\"{}\">\n",
        escape_html(p.canonical)
    ));
    if !site.is_empty() {
        h.push_str(&format!(
            "<meta property=\"og:site_name\" content=\"{}\">\n",
            escape_html(site)
        ));
    }
    if let Some(img) = p.image {
        h.push_str(&format!(
            "<meta property=\"og:image\" content=\"{}\">\n",
            escape_html(img)
        ));
    }
    h.push_str(&format!(
        "<meta name=\"twitter:card\" content=\"{}\">\n",
        if p.image.is_some() {
            "summary_large_image"
        } else {
            "summary"
        }
    ));
    h.push_str(&format!(
        "<meta name=\"twitter:title\" content=\"{}\">\n",
        escape_html(p.title)
    ));
    if has_desc {
        h.push_str(&format!(
            "<meta name=\"twitter:description\" content=\"{}\">\n",
            escape_html(p.description)
        ));
    }
    if let Some(img) = p.image {
        h.push_str(&format!(
            "<meta name=\"twitter:image\" content=\"{}\">\n",
            escape_html(img)
        ));
    }
    h.push_str(&format!(
        "<script type=\"application/ld+json\">{}</script>\n",
        json_ld_script(p.json_ld)
    ));
    h.push_str("</head>\n<body>\n");
    h.push_str(&format!("<h1>{}</h1>\n", escape_html(p.title)));
    if has_desc {
        h.push_str(&format!("<p>{}</p>\n", escape_html(p.description)));
    }
    h.push_str(&format!(
        "<p><a href=\"{}\">阅读全文</a></p>\n",
        escape_html(p.canonical)
    ));
    h.push_str("</body>\n</html>\n");
    h
}

/// JSON-LD 序列化：serde_json → 再把 `<`/`>`/`&` 换成 JSON unicode 转义，
/// 防 `</script>` 提前截断（`<script>` 内 HTML 实体转义无效，必须用 JSON 层转义）
fn json_ld_script(v: &serde_json::Value) -> String {
    let text = serde_json::to_string(v).unwrap_or_else(|_| "{}".to_string());
    text.replace('<', "\\u003c")
        .replace('>', "\\u003e")
        .replace('&', "\\u0026")
}

/// 文章 JSON-LD（BlogPosting）入参
pub struct PostJsonLd<'a> {
    pub title: &'a str,
    pub description: &'a str,
    pub canonical: &'a str,
    pub site_name: &'a str,
    pub published_at: &'a str,
    pub updated_at: &'a str,
    pub image: Option<&'a str>,
    pub word_count: usize,
}

/// 构造文章 BlogPosting JSON-LD（字段可空时省略，保证合法 JSON-LD）
pub fn post_json_ld(d: &PostJsonLd<'_>) -> serde_json::Value {
    let mut v = serde_json::json!({
        "@context": "https://schema.org",
        "@type": "BlogPosting",
        "headline": d.title,
        "description": d.description,
        "url": d.canonical,
        "mainEntityOfPage": {"@type": "WebPage", "@id": d.canonical},
        "author": {"@type": "Organization", "name": d.site_name},
        "publisher": {"@type": "Organization", "name": d.site_name},
        "wordCount": d.word_count,
    });
    let obj = v.as_object_mut().expect("json! 构造的对象");
    if !d.published_at.trim().is_empty() {
        obj.insert("datePublished".into(), serde_json::json!(d.published_at));
    }
    let modified = if d.updated_at.trim().is_empty() {
        d.published_at
    } else {
        d.updated_at
    };
    if !modified.trim().is_empty() {
        obj.insert("dateModified".into(), serde_json::json!(modified));
    }
    if let Some(img) = d.image {
        obj.insert("image".into(), serde_json::json!(img));
    }
    v
}

/// 页面 JSON-LD（WebPage）入参
pub struct PageJsonLd<'a> {
    pub title: &'a str,
    pub description: &'a str,
    pub canonical: &'a str,
    pub site_name: &'a str,
    /// 站点根（无尾 `/`；isPartOf.url 用）
    pub base: &'a str,
    pub updated_at: &'a str,
    pub image: Option<&'a str>,
}

/// 构造页面 WebPage JSON-LD
pub fn page_json_ld(d: &PageJsonLd<'_>) -> serde_json::Value {
    let mut v = serde_json::json!({
        "@context": "https://schema.org",
        "@type": "WebPage",
        "name": d.title,
        "description": d.description,
        "url": d.canonical,
        "isPartOf": {
            "@type": "WebSite",
            "name": d.site_name,
            "url": format!("{}/", d.base.trim_end_matches('/')),
        },
    });
    let obj = v.as_object_mut().expect("json! 构造的对象");
    if !d.updated_at.trim().is_empty() {
        obj.insert("dateModified".into(), serde_json::json!(d.updated_at));
    }
    if let Some(img) = d.image {
        obj.insert("image".into(), serde_json::json!(img));
    }
    v
}

/// robots.txt 内容（契约「SEO / 分享元信息」）：放行全站、封后台与后台 API、声明 sitemap
pub fn render_robots_txt(base: &str) -> String {
    format!(
        "User-agent: *\nAllow: /\nDisallow: /admin\nDisallow: /api/admin\n\n\
         Sitemap: {}/api/sitemap.xml\n",
        base.trim_end_matches('/')
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crawler_ua_whitelist() {
        for ua in [
            "Twitterbot/1.0",
            "Mozilla/5.0 (compatible; Googlebot/2.1; +http://www.google.com/bot.html)",
            "facebookexternalhit/1.1 (+http://www.facebook.com/externalhit_uatext.php)",
            "Mozilla/5.0 (compatible; Baiduspider/2.0)",
            "Mozilla/5.0 (Linux; Android) MicroMessenger/8.0",
            "Slackbot-LinkExpanding 1.0",
            "TelegramBot (like TwitterBot)",
            "WhatsApp/2.23",
            "Discordbot/2.0",
            "Mozilla/5.0 AppleWebKit/537.36 Chrome/120 BingPreview/1.0b",
            "some-unknown-crawler/1.0",
        ] {
            assert!(is_crawler_ua(ua), "应命中白名单: {ua}");
        }
        // 普通浏览器 / 命令行工具不误伤（curl 未命中 → 走 302）
        for ua in [
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
             (KHTML, like Gecko) Chrome/126.0 Safari/537.36",
            "Mozilla/5.0 (iPhone; CPU iPhone OS 17_0 like Mac OS X) AppleWebKit/605.1.15",
            "curl/8.4.0",
            "",
        ] {
            assert!(!is_crawler_ua(ua), "不应命中白名单: {ua}");
        }
    }

    #[test]
    fn escapes_html_special_chars() {
        assert_eq!(
            escape_html(r#"<script>alert("x") & 'y'</script>"#),
            "&lt;script&gt;alert(&quot;x&quot;) &amp; &#39;y&#39;&lt;/script&gt;"
        );
        assert_eq!(escape_html("中文标题"), "中文标题");
    }

    #[test]
    fn extracts_first_markdown_image() {
        assert_eq!(
            first_image_url("正文\n\n![图](/api/uploads/a/b.png)\n\n![第二](c.png)"),
            Some("/api/uploads/a/b.png".to_string())
        );
        // <url> 包裹与 title 后缀
        assert_eq!(
            first_image_url(r#"![x](<https://cdn.example.com/a b.png> "标题")"#),
            Some("https://cdn.example.com/a b.png".to_string())
        );
        assert_eq!(
            first_image_url(r#"![x](https://cdn.example.com/a.png "标题")"#),
            Some("https://cdn.example.com/a.png".to_string())
        );
        // 代码围栏内的图片不算
        assert_eq!(
            first_image_url("```\n![x](in-code.png)\n```\n\n![y](real.png)"),
            Some("real.png".to_string())
        );
        // 只有围栏内图片 → 无
        assert_eq!(first_image_url("```\n![x](in-code.png)\n```"), None);
        // 普通链接不是图片
        assert_eq!(first_image_url("[链接](https://x.dev)"), None);
        assert_eq!(first_image_url(""), None);
    }

    #[test]
    fn absolutizes_relative_urls() {
        assert_eq!(
            absolutize("https://blog.example.com", "/api/uploads/a.png"),
            Some("https://blog.example.com/api/uploads/a.png".to_string())
        );
        assert_eq!(
            absolutize("https://blog.example.com", "https://cdn.x.dev/a.png"),
            Some("https://cdn.x.dev/a.png".to_string())
        );
        assert_eq!(
            absolutize("https://blog.example.com", "images/a.png"),
            Some("https://blog.example.com/images/a.png".to_string())
        );
        assert_eq!(absolutize("not a base", "/a.png"), None);
        assert_eq!(absolutize("https://x.dev", "  "), None);
    }

    #[test]
    fn renders_og_html_with_escaping() {
        let json_ld = post_json_ld(&PostJsonLd {
            title: r#"标题 <script>alert("x")</script>"#,
            description: "描述 & 更多",
            canonical: "https://x.dev/posts/a",
            site_name: "站点",
            published_at: "2026-10-01T00:00:00Z",
            updated_at: "2026-10-02T00:00:00Z",
            image: Some("https://x.dev/a.png"),
            word_count: 42,
        });
        let html = render_og_html(&OgPage {
            title: r#"标题 <script>alert("x")</script>"#,
            description: "描述 & 更多",
            canonical: "https://x.dev/posts/a",
            base: "https://x.dev/",
            site_name: "站点",
            og_type: "article",
            image: Some("https://x.dev/a.png"),
            json_ld: &json_ld,
        });
        assert!(html.contains(
            "<title>标题 &lt;script&gt;alert(&quot;x&quot;)&lt;/script&gt; - 站点</title>"
        ));
        // RSS 自动发现：base 尾斜杠不会造成双斜杠
        assert!(html.contains(
            r#"<link rel="alternate" type="application/rss+xml" href="https://x.dev/api/feed.xml">"#
        ));
        assert!(html.contains(r#"<meta property="og:description" content="描述 &amp; 更多">"#));
        assert!(html.contains(r#"<meta property="og:image" content="https://x.dev/a.png">"#));
        assert!(html.contains(r#"content="summary_large_image""#));
        // 原始 <script> 不得出现；JSON-LD 内 < 已转 unicode 转义
        assert!(!html.contains("<script>alert"));
        assert!(html.contains("\\u003cscript"));
        assert!(html.contains("application/ld+json"));
    }

    #[test]
    fn renders_og_html_without_image() {
        let json_ld = page_json_ld(&PageJsonLd {
            title: "关于",
            description: "",
            canonical: "https://x.dev/pages/about",
            site_name: "站点",
            base: "https://x.dev",
            updated_at: "",
            image: None,
        });
        let html = render_og_html(&OgPage {
            title: "关于",
            description: "",
            canonical: "https://x.dev/pages/about",
            base: "https://x.dev",
            site_name: "站点",
            og_type: "website",
            image: None,
            json_ld: &json_ld,
        });
        assert!(!html.contains("og:image"));
        assert!(!html.contains("twitter:image"));
        assert!(html.contains(r#"content="summary""#));
        assert!(!html.contains(r#"<meta name="description""#));
    }

    #[test]
    fn robots_txt_follows_contract() {
        let txt = render_robots_txt("https://x.dev/");
        assert!(txt.contains("User-agent: *"));
        assert!(txt.contains("Allow: /"));
        assert!(txt.contains("Disallow: /admin\n"));
        assert!(txt.contains("Disallow: /api/admin"));
        assert!(txt.contains("Sitemap: https://x.dev/api/sitemap.xml"));
    }
}
