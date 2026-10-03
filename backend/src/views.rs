//! 浏览量计数辅助（契约「浏览量与点赞」条款）：
//! - `ViewDedup`：进程内 (ip, post_id) 去重表，60 分钟窗口内同一来源对同一文章
//!   只计一次。内存态、重启清零——契约明确「浏览量尽力去重、非精确审计」，
//!   不引入 Redis 等外部依赖；
//! - `is_bot_ua`：常见爬虫 UA 关键字过滤（命中不计数）；
//! - `client_ip`：去重键的 IP 来源：X-Forwarded-For 首项 → X-Real-IP →
//!   TCP 直连地址（生产路径 run() 启用 ConnectInfo）→ 兜底 "direct"。

use axum::extract::ConnectInfo;
use axum::http::HeaderMap;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

/// 去重窗口：同一 (ip, post_id) 60 分钟内不重复计数（常量便于测试引用）
pub const VIEW_DEDUP_WINDOW: Duration = Duration::from_secs(60 * 60);

/// 条目软上限：达到后在下一次写入前整表清扫过期项，防内存无限增长
/// （个人博客量级下几乎不会触发；触发也只是 O(n) 清扫一次）
const MAX_ENTRIES: usize = 50_000;

/// 进程内浏览量去重表：key = "{ip}|{post_id}"，value = 最近计数时刻
#[derive(Debug)]
pub struct ViewDedup {
    inner: Mutex<HashMap<String, Instant>>,
    window: Duration,
}

impl Default for ViewDedup {
    fn default() -> Self {
        Self::new(VIEW_DEDUP_WINDOW)
    }
}

impl ViewDedup {
    pub fn new(window: Duration) -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
            window,
        }
    }

    /// 首次命中（或上次计数已超出窗口）返回 true（应计数）并记录时刻；
    /// 窗口内重复命中返回 false（不计数）。锁中毒时恢复继续（去重是尽力而为语义）
    pub fn should_count(&self, ip: &str, post_id: i64) -> bool {
        let key = format!("{ip}|{post_id}");
        let now = Instant::now();
        let mut map = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        if map.len() >= MAX_ENTRIES {
            map.retain(|_, t| now.duration_since(*t) < self.window);
        }
        match map.get(&key) {
            Some(t) if now.duration_since(*t) < self.window => false,
            _ => {
                map.insert(key, now);
                true
            }
        }
    }
}

/// 常见爬虫/工具 UA 关键字（小写子串匹配；契约：爬虫 UA 不计数）
const BOT_UA_KEYWORDS: [&str; 16] = [
    "bot",
    "crawl",
    "spider",
    "slurp",
    "curl",
    "wget",
    "python",
    "httpclient",
    "okhttp",
    "headless",
    "scanner",
    "preview",
    "facebookexternalhit",
    "mediapartners",
    "feedfetcher",
    "lighthouse",
];

/// 是否为爬虫/自动化工具 UA（大小写不敏感子串匹配）
pub fn is_bot_ua(ua: &str) -> bool {
    let lower = ua.to_ascii_lowercase();
    BOT_UA_KEYWORDS.iter().any(|k| lower.contains(k))
}

/// 请求是否带 Bearer 凭据（后台管理端预览不计数；只认头部存在性，不校验有效性——
/// 无效 token 在公开接口本就允许访问，但按「后台请求」对待不计浏览）
pub fn has_bearer(headers: &HeaderMap) -> bool {
    headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .map(|v| {
            v.strip_prefix("Bearer ")
                .map(|t| !t.trim().is_empty())
                .unwrap_or(false)
        })
        .unwrap_or(false)
}

/// 去重键的客户端 IP：X-Forwarded-For 首项（反代链最早的客户端）→ X-Real-IP →
/// TCP 直连地址（仅生产路径提供 ConnectInfo）→ 兜底 "direct"（无从区分来源时
/// 视为同一来源，尽力去重语义）
pub fn client_ip(headers: &HeaderMap, connect: Option<&ConnectInfo<SocketAddr>>) -> String {
    if let Some(xff) = headers
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(',').next())
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        return xff.to_string();
    }
    if let Some(real) = headers
        .get("x-real-ip")
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        return real.to_string();
    }
    if let Some(ConnectInfo(addr)) = connect {
        return addr.ip().to_string();
    }
    "direct".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dedup_blocks_within_window_and_expires_after() {
        let dedup = ViewDedup::new(Duration::from_millis(50));
        assert!(dedup.should_count("1.1.1.1", 1), "首次应计数");
        assert!(!dedup.should_count("1.1.1.1", 1), "窗口内重复不计数");
        // 不同文章 / 不同 IP 互不影响
        assert!(dedup.should_count("1.1.1.1", 2));
        assert!(dedup.should_count("2.2.2.2", 1));
        // 超出窗口后重新计数
        std::thread::sleep(Duration::from_millis(80));
        assert!(dedup.should_count("1.1.1.1", 1), "窗口过期后应再次计数");
    }

    #[test]
    fn default_window_is_60_minutes() {
        assert_eq!(VIEW_DEDUP_WINDOW, Duration::from_secs(3600));
    }

    #[test]
    fn bot_ua_detection() {
        assert!(is_bot_ua("Mozilla/5.0 (compatible; Googlebot/2.1)"));
        assert!(is_bot_ua("Mozilla/5.0 (Linux; Android) BingPreview/1.0b"));
        assert!(is_bot_ua("curl/8.4.0"));
        assert!(is_bot_ua("Python-urllib/3.12"));
        assert!(is_bot_ua("Baiduspider/2.0"));
        // 正常浏览器 UA 不误伤
        assert!(!is_bot_ua(
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
             (KHTML, like Gecko) Chrome/126.0 Safari/537.36"
        ));
        assert!(!is_bot_ua(
            "Mozilla/5.0 (iPhone; CPU iPhone OS 17_0 like Mac OS X) AppleWebKit/605.1.15"
        ));
        assert!(!is_bot_ua(""));
    }

    #[test]
    fn bearer_detection() {
        let mut h = HeaderMap::new();
        assert!(!has_bearer(&h));
        h.insert(
            axum::http::header::AUTHORIZATION,
            "Bearer abc.def.ghi".parse().unwrap(),
        );
        assert!(has_bearer(&h));
        h.insert(
            axum::http::header::AUTHORIZATION,
            "Basic dXNlcjpwYXNz".parse().unwrap(),
        );
        assert!(!has_bearer(&h));
        h.insert(
            axum::http::header::AUTHORIZATION,
            "Bearer ".parse().unwrap(),
        );
        assert!(!has_bearer(&h), "空 token 不算 Bearer 请求");
    }

    #[test]
    fn client_ip_priority() {
        let mut h = HeaderMap::new();
        // 无任何来源信息 → 兜底
        assert_eq!(client_ip(&h, None), "direct");
        // ConnectInfo 兜底前生效
        let conn = ConnectInfo("192.168.1.9:54321".parse::<SocketAddr>().unwrap());
        assert_eq!(client_ip(&h, Some(&conn)), "192.168.1.9");
        // X-Real-IP 优先于 ConnectInfo
        h.insert("x-real-ip", "10.0.0.1".parse().unwrap());
        assert_eq!(client_ip(&h, Some(&conn)), "10.0.0.1");
        // XFF 首项优先于一切（反代链最早的客户端）
        h.insert("x-forwarded-for", "1.2.3.4, 10.0.0.2".parse().unwrap());
        assert_eq!(client_ip(&h, Some(&conn)), "1.2.3.4");
    }
}
